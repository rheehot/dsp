; ModuleID = 'tests\BareMinimum\test.py'
source_filename = "tests\\BareMinimum\\test.py"
target datalayout = "e-P1-p:16:8-i8:8-i16:8-i32:8-i64:8-f32:8-f64:8-n8-a:8"
target triple = "avr"

declare void @pinMode(i8, i8) addrspace(1)

declare void @delay(i32) addrspace(1)

declare void @digitalWrite(i8, i8) addrspace(1)

define void @setup() addrspace(1) {
  ret void
}

define void @loop() addrspace(1) {
  ret void
}
