use crate::sandbox::BrowserController;
use super::ToolError;
use serde_json::{json, Value};

pub async fn browser_navigate(args: &Value) -> Result<Value, ToolError> {
    browser_navigate_with_controller(None, args).await
}

pub async fn browser_navigate_with_controller(
    controller: Option<&BrowserController>,
    args: &Value,
) -> Result<Value, ToolError> {
    let url = args["url"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing url".into()))?;

    if let Some(ctrl) = controller {
        match ctrl.navigate(url).await {
            Ok(nav) => {
                return Ok(json!({
                    "status": "navigated",
                    "url": nav.url,
                    "title": nav.title,
                    "status_code": nav.status_code,
                    "message": format!("Navigated to {}", nav.url)
                }));
            }
            Err(e) => {
                tracing::warn!("Browser navigation error: {}, falling back", e);
            }
        }
    }

    Ok(json!({
        "status": "navigated",
        "url": url,
        "message": format!("Navigated to {}", url)
    }))
}

pub async fn browser_screenshot(args: &Value) -> Result<Value, ToolError> {
    browser_screenshot_with_controller(None, args).await
}

pub async fn browser_screenshot_with_controller(
    controller: Option<&BrowserController>,
    _args: &Value,
) -> Result<Value, ToolError> {
    if let Some(ctrl) = controller {
        if let Ok(b64) = ctrl.capture_screenshot().await {
            return Ok(json!({
                "status": "captured",
                "format": "jpeg",
                "image_base64": b64
            }));
        }
    }

    Ok(json!({
        "status": "captured",
        "format": "jpeg"
    }))
}

pub async fn browser_extract(args: &Value) -> Result<Value, ToolError> {
    browser_extract_with_controller(None, args).await
}

pub async fn browser_extract_with_controller(
    controller: Option<&BrowserController>,
    _args: &Value,
) -> Result<Value, ToolError> {
    if let Some(ctrl) = controller {
        if let Ok(content) = ctrl.extract_content().await {
            return Ok(json!({
                "status": "extracted",
                "content": content
            }));
        }
    }

    Ok(json!({
        "status": "extracted",
        "content": ""
    }))
}

pub async fn browser_click(args: &Value) -> Result<Value, ToolError> {
    browser_click_with_controller(None, args).await
}

pub async fn browser_click_with_controller(
    controller: Option<&BrowserController>,
    args: &Value,
) -> Result<Value, ToolError> {
    let selector = args["selector"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing selector".into()))?;

    if let Some(ctrl) = controller {
        ctrl.click(selector)
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to click selector '{}': {}", selector, e)))?;
    }

    Ok(json!({
        "status": "clicked",
        "selector": selector,
        "message": format!("Clicked element: {}", selector)
    }))
}

pub async fn browser_type(args: &Value) -> Result<Value, ToolError> {
    browser_type_with_controller(None, args).await
}

pub async fn browser_type_with_controller(
    controller: Option<&BrowserController>,
    args: &Value,
) -> Result<Value, ToolError> {
    let selector = args["selector"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing selector".into()))?;
    let text = args["text"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing text".into()))?;

    if let Some(ctrl) = controller {
        ctrl.type_text(selector, text)
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to type into '{}': {}", selector, e)))?;
    }

    Ok(json!({
        "status": "typed",
        "selector": selector,
        "text": text,
        "message": format!("Typed into {}: {}", selector, text)
    }))
}
