use crate::BrowserError;
use serde_json::{Value, json};

pub(crate) fn runtime_evaluate_params(expression: &str, include_command_line_api: bool) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("expression".to_owned(), json!(expression));
    params.insert("returnByValue".to_owned(), json!(true));
    params.insert("awaitPromise".to_owned(), json!(true));
    if include_command_line_api {
        params.insert("includeCommandLineAPI".to_owned(), json!(true));
    }
    Value::Object(params)
}

pub(crate) fn runtime_evaluate_value(result: Value) -> Result<Value, BrowserError> {
    runtime_command_value(result, "Runtime.evaluate")
}

pub(crate) fn runtime_command_value(result: Value, method: &str) -> Result<Value, BrowserError> {
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(BrowserError::CommandFailed {
            method: method.to_owned(),
            message: runtime_exception_message(exception, "runtime command exception"),
        });
    }

    result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| BrowserError::MissingResponseData(format!("{method} value")))
}

fn runtime_exception_message(exception: &Value, fallback: &str) -> String {
    exception
        .get("exception")
        .and_then(|exception| exception.get("description"))
        .and_then(Value::as_str)
        .or_else(|| exception.get("text").and_then(Value::as_str))
        .or_else(|| {
            exception
                .get("exception")
                .and_then(|exception| exception.get("value"))
                .and_then(Value::as_str)
        })
        .unwrap_or(fallback)
        .to_owned()
}

pub(crate) fn render_runtime_evaluate_result(result: &Value) -> Result<String, BrowserError> {
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(BrowserError::CommandFailed {
            method: "Runtime.evaluate".to_owned(),
            message: runtime_exception_message(exception, "Runtime.evaluate exception"),
        });
    }

    let result = result
        .get("result")
        .ok_or_else(|| BrowserError::MissingResponseData("Runtime.evaluate result".to_owned()))?;

    if result.get("wasThrown").and_then(Value::as_bool) == Some(true) {
        return Err(BrowserError::CommandFailed {
            method: "Runtime.evaluate".to_owned(),
            message: result
                .get("description")
                .or_else(|| result.get("value"))
                .map(render_json_value)
                .unwrap_or_else(|| "JavaScript execution failed".to_owned()),
        });
    }

    if let Some(value) = result.get("value") {
        return Ok(render_json_value(value));
    }

    if let Some(unserializable) = result.get("unserializableValue").and_then(Value::as_str) {
        return Ok(unserializable.to_owned());
    }

    if result.get("type").and_then(Value::as_str) == Some("undefined") {
        return Ok("undefined".to_owned());
    }

    if let Some(description) = result.get("description").and_then(Value::as_str) {
        return Ok(description.to_owned());
    }

    Err(BrowserError::MissingResponseData(
        "Runtime.evaluate rendered value".to_owned(),
    ))
}

fn render_json_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
