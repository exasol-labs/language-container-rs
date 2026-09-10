use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

struct Spy {
    emitted: Vec<Vec<Value>>,
}

impl UdfContext for Spy {
    fn num_columns(&self) -> usize {
        0
    }
    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Unimplemented("spy".into()))
    }
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        self.emitted.push(values.to_vec());
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

#[test]
fn emit_owned_default_forwards_to_emit() {
    let mut spy = Spy {
        emitted: Vec::new(),
    };
    spy.emit_owned(vec![Value::Int64(42)]).unwrap();
    assert_eq!(spy.emitted.len(), 1);
    assert_eq!(spy.emitted[0], vec![Value::Int64(42)]);
}
