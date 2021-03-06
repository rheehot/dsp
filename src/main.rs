use std::error::Error;
use std::fs::{read_to_string, remove_file};
use std::io::Write;
use std::process::Command;

use clap::{App, Arg, ArgMatches};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::{PassManager, PassManagerBuilder};
use inkwell::targets::{TargetData, TargetTriple};
use inkwell::OptimizationLevel;
use rustpython_parser::parser::parse_program;

mod compiler;
mod irgen;
mod value;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const AUTHORS: &'static str = env!("CARGO_PKG_AUTHORS");

fn parse_arguments<'a>(app: App<'a, '_>) -> ArgMatches<'a> {
    let mut usage = app.get_name().to_string();
    usage.push_str(" <command> <file> [-p PORT] [--emit-llvm] [--no-opt]");
    app.usage(&*usage)
        .arg(
            Arg::with_name("command")
                .index(1)
                .required(true)
                .help("Command to execute {build,flash}"),
        )
        .arg(
            Arg::with_name("file")
                .index(2)
                .required(true)
                .help("Source file path"),
        )
        .arg(
            Arg::with_name("emit-llvm")
                .long("emit-llvm")
                .help("Emits llvm ir"),
        )
        .arg(
            Arg::with_name("no-opt")
                .long("no-opt")
                .help("Disables optimization"),
        )
        .arg(
            Arg::with_name("port")
                .takes_value(true)
                .short("p")
                .long("port")
                .help("Serial port to flash"),
        )
        .get_matches()
}

fn build<'a, 'ctx>(pkg: &str, no_opt: bool) -> String {
    let ctx = Context::create();
    let module = ctx.create_module(pkg);

    // Create target data structure for Arduino
    let target_data = TargetData::create("e-P1-p:16:8-i8:8-i16:8-i32:8-i64:8-f32:8-f64:8-n8-a:8");
    module.set_data_layout(&target_data.get_data_layout());

    // LLVM triple
    module.set_triple(&TargetTriple::create("avr"));

    // Initialize pass manager
    let pm: PassManager<Module> = PassManager::create(());
    let pm_builder = PassManagerBuilder::create();
    pm_builder.set_optimization_level(OptimizationLevel::Aggressive);
    pm_builder.populate_module_pass_manager(&pm);

    // Read source code from package file
    let python_source = read_to_string(pkg).unwrap();

    // Parse the source code
    let program = parse_program(&python_source).unwrap();

    // Create a root builder context
    let builder = ctx.create_builder();

    let mut c = compiler::Compiler::new(String::from(pkg), &ctx, &builder, &module);
    c.compile(program);

    // Run passes if no_opt flag is off
    if !no_opt {
        pm.run_on(&c.module);
    }

    // LLVM assembly path
    let assembly = String::from(pkg) + ".ll";

    // Write assembly to file
    c.module.print_to_file(&assembly).unwrap();

    {
        assembly
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let app = App::new("dsp" /* TODO: dspython */)
        .version(VERSION)
        .author(AUTHORS)
        .about("Damn Small Python is a Python compiler for Arduino");

    // Parse command-line arguments
    let matches = parse_arguments(app);

    let command = matches.value_of("command").unwrap();
    let file = matches.value_of("file").unwrap();
    let emit_llvm = matches.is_present("emit-llvm");
    let no_opt = matches.is_present("no-opt");

    // Load the environmental variable: `ARDUINO_DIR`
    let _arduino_dir = std::env::var("ARDUINO_DIR").expect(
        "You must set the environment variable 'ARDUINO_DIR' as your arduino software location",
    );
    let arduino_dir = _arduino_dir.as_str();

    // Check if command is valid
    if !(command == "build" || command == "flash" || command == "upload") {
        panic!(format!("Unknown command '{}'", command));
    }

    // Build assembly from python file
    let assembly = build(file, no_opt);

    // Linker path
    let linker = if cfg!(debug_assertions) {
        "python builder/builder.py"
    } else {
        "bin/builder"
    };

    // Execute the linker command
    let out = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(&["/C", linker, arduino_dir, assembly.as_str()])
            .status()
            .expect("Failed to execute command")
    } else {
        Command::new("sh")
            .args(&["-c", linker, arduino_dir, assembly.as_str()])
            .status()
            .expect("Failed to execute command")
    };
    if !out.success() {
        panic!("Failed to perform builder.");
    }

    if command == "flash" || command == "upload" {
        let port = matches.value_of("port").expect("Port not provided!");
        print!("{} << {}...", port, file);
        std::io::stdout().flush().unwrap_or_default();

        // Uploader path
        let uploader = if cfg!(debug_assertions) {
            "python scripts/flash.py"
        } else {
            "bin/flash"
        };

        // Hex file path
        let hex_file = &(assembly.to_owned() + ".hex");

        // Execute the uploader command
        let out = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(&["/C", uploader, arduino_dir, hex_file, port])
                .output()
                .expect("Failed to execute command")
        } else {
            Command::new("sh")
                .args(&["-c", uploader, arduino_dir, hex_file, port])
                .output()
                .expect("Failed to execute command")
        };
        if !out.status.success() {
            println!("{}", String::from_utf8_lossy(&out.stderr));
            panic!("Failed to perform uploading.");
        }
        // Remove the hex file after finishing upload
        remove_file(hex_file).unwrap();
        println!("[Done]")
    }

    // Remove the assembly file if flag `emit-llvm` is not enabled
    if !emit_llvm {
        remove_file(&assembly).unwrap();
    }

    Ok(())
}
