use either::Either;
use inkwell::values::BasicValueEnum;
use rustpython_parser::ast;

use crate::compiler::error::{CompilerErrorReport, CompilerErrorType};
use crate::compiler::mangle::mangling;
use crate::compiler::Compiler;
use crate::value::convert::{truncate_bigint_to_u64, try_get_constant_string};
use crate::value::{Value, ValueType};

pub trait CGExpr<'a, 'ctx> {
    fn compile_expr(&mut self, expr: &ast::Expression) -> Value<'ctx>;
    fn compile_expr_call(
        &mut self,
        func: &Box<ast::Expression>,
        args: &Vec<ast::Expression>,
    ) -> Value<'ctx>;
}

impl<'a, 'ctx> CGExpr<'a, 'ctx> for Compiler<'a, 'ctx> {
    fn compile_expr(&mut self, expr: &ast::Expression) -> Value<'ctx> {
        self.set_source_location(expr.location);

        use rustpython_parser::ast::ExpressionType;
        match &expr.node {
            ExpressionType::Number { value } => match value {
                ast::Number::Integer { value } => Value::I16 {
                    value: self
                        .context
                        .i16_type()
                        .const_int(truncate_bigint_to_u64(value), true),
                },
                ast::Number::Float { value } => Value::F32 {
                    value: self.context.f32_type().const_float(*value),
                },
                ast::Number::Complex { real: _, imag: _ } => {
                    panic!(
                        "{:?}\nNotImplemented builder for imaginary number",
                        self.current_source_location
                    );
                }
            },
            ExpressionType::String { value } => {
                let v = try_get_constant_string(value).unwrap();
                if self.fn_value_opt.is_some() {
                    Value::Str {
                        value: self
                            .builder
                            .build_global_string_ptr(v.as_str(), ".str")
                            .as_pointer_value(),
                    }
                } else {
                    // TODO: Global string builder
                    panic!("NotImplemented builder for global string")
                }
            }
            ExpressionType::Call {
                function,
                args,
                keywords,
            } => {
                let _keywords = keywords;
                self.compile_expr_call(function, args)
            }
            ExpressionType::Binop { a, op, b } => {
                let a = self.compile_expr(a);
                let b = self.compile_expr(b);
                self.compile_op(a, op, b)
            }
            ExpressionType::Identifier { name } => {
                if self.fn_value_opt.is_none() {
                    let (ty, pointer) = match self.variables.get(name) {
                        Some(tuple) => tuple,
                        None => panic!(self.errs(CompilerErrorType::NameError(name))),
                    };
                    match *pointer {
                        var => {
                            Value::from_basic_value(*ty, self.builder.build_load(var, name).into())
                        }
                    }
                } else {
                    let getter = self.fn_scope.get(&self.fn_value()).unwrap().get(name);
                    if getter.is_none() {
                        let (ty, pointer) = match self.variables.get(name) {
                            Some(tuple) => tuple,
                            None => panic!(self.errs(CompilerErrorType::NameError(name))),
                        };
                        match *pointer {
                            var => Value::from_basic_value(
                                *ty,
                                self.builder.build_load(var, name).into(),
                            ),
                        }
                    } else {
                        let (ty, pointer) = getter.unwrap();
                        match *pointer {
                            var => Value::from_basic_value(
                                *ty,
                                self.builder.build_load(var, name).into(),
                            ),
                        }
                    }
                }
            }
            ExpressionType::Compare { vals, ops } => self.compile_comparison(vals, ops),
            ExpressionType::None => Value::Void,
            ExpressionType::True => Value::Bool {
                value: self.context.bool_type().const_int(1, false),
            },
            ExpressionType::False => Value::Bool {
                value: self.context.bool_type().const_int(0, false),
            },
            ExpressionType::Unop { op, a } => match &a.node {
                ExpressionType::Number { value } => match value {
                    ast::Number::Integer { value } => match op {
                        ast::UnaryOperator::Neg => Value::I16 {
                            value: self
                                .context
                                .i16_type()
                                .const_int(truncate_bigint_to_u64(&-value), true),
                        },
                        _ => panic!("NotImplemented unop for i16"),
                    },
                    ast::Number::Float { value } => match op {
                        ast::UnaryOperator::Neg => Value::F32 {
                            value: self.context.f32_type().const_float(-value.clone()),
                        },
                        _ => panic!("NotImplemented unop for f32"),
                    },
                    ast::Number::Complex { real: _, imag: _ } => {
                        panic!(
                            "{:?}\nNotImplemented builder for imaginary number",
                            self.current_source_location
                        );
                    }
                },
                _ => panic!("NotImplemented type for unop"),
            },
            ExpressionType::Ellipsis => panic!("Constant value Ellipsis is not implemented."),
            _ => {
                panic!(
                    "{:?}\nNotImplemented expression {:?}",
                    self.current_source_location, expr.node,
                );
            }
        }
    }

    fn compile_expr_call(
        &mut self,
        func: &Box<ast::Expression>,
        args: &Vec<ast::Expression>,
    ) -> Value<'ctx> {
        let func_name = match &func.node {
            ast::ExpressionType::Identifier { name } => name,
            _ => {
                panic!(
                    "{:?}\nUnknown function name {:?}",
                    self.current_source_location, func.node
                );
            }
        }
        .to_string();

        let first_arg = self.compile_expr(args.clone().first().unwrap());

        let func = match self.get_function(func_name.as_ref()) {
            Some(f) => f,
            None => {
                let func_name_mangled = mangling(&func_name, first_arg.get_type());
                self.get_function(func_name_mangled.as_ref()).expect(
                    format!(
                        "{:?}\nFunction '{}' is not defined",
                        self.current_source_location, func_name
                    )
                    .as_str(),
                )
            }
        };

        let args_proto = func.get_params();

        let mut args_value: Vec<BasicValueEnum> = vec![];

        for (i, expr_proto) in args.iter().zip(args_proto.iter()).enumerate() {
            let expr = expr_proto.0;
            let proto = expr_proto.1;
            let value = if i == 0 {
                first_arg
            } else {
                self.compile_expr(expr)
            };
            match value {
                Value::I8 { value } => {
                    let cast = self.builder.build_int_cast(
                        value,
                        proto.get_type().into_int_type(),
                        "icast",
                    );
                    args_value.push(BasicValueEnum::IntValue(cast))
                }
                Value::I16 { value } => {
                    let cast = self.builder.build_int_truncate(
                        value,
                        proto.get_type().into_int_type(),
                        "itrunc",
                    );
                    args_value.push(BasicValueEnum::IntValue(cast))
                }
                Value::F32 { value } => args_value.push(BasicValueEnum::FloatValue(value)),
                Value::Str { value } => args_value.push(BasicValueEnum::PointerValue(value)),
                _ => panic!(
                    "{:?}\nNotImplemented argument type",
                    self.current_source_location
                ),
            }
        }

        let res = self.builder.build_call(func, args_value.as_slice(), "call");
        res.set_tail_call(true);

        match res.try_as_basic_value() {
            // Return type
            Either::Left(bv) => Value::from_basic_value(
                if bv.is_int_value() {
                    let iv = bv.into_int_value();

                    match iv.get_type().get_bit_width() {
                        8 => ValueType::I8,
                        16 => ValueType::I16,
                        _ => unreachable!(),
                    }
                } else if bv.is_float_value() {
                    ValueType::F32
                } else {
                    unreachable!()
                },
                bv,
            ),
            Either::Right(_) => Value::Void,
        }
    }
}
