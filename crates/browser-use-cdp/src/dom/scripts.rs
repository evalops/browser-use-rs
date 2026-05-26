use super::*;

mod interactive_elements;

pub(crate) const AX_REF_ATTRIBUTE: &str = "data-browser-use-rs-ax-ref";
#[cfg(test)]
pub(crate) use interactive_elements::INTERACTIVE_ELEMENTS_JS;
pub(crate) use interactive_elements::{
    CLEANUP_AX_REFS_JS, FRAME_ELEMENTS_JS, PAGE_INFO_JS, interactive_elements_js,
};

pub(crate) fn element_eval_js(index: u32, body: &str) -> String {
    format!(
        r#"
(() => {{
  const selector = [
    'a',
    'button',
    'input',
    'textarea',
    'select',
    'details',
    'summary',
    'audio[controls]',
    'video[controls]',
    'option',
    'optgroup',
    '[role="button"]',
    '[role="link"]',
    '[role="menuitem"]',
    '[role="option"]',
    '[role="radio"]',
    '[role="checkbox"]',
    '[role="tab"]',
    '[role="textbox"]',
    '[role="combobox"]',
    '[role="listbox"]',
    '[role="slider"]',
    '[role="spinbutton"]',
    '[role="search"]',
    '[role="searchbox"]',
    '[role="row"]',
    '[role="cell"]',
    '[role="gridcell"]',
    '[onclick]',
    '[onmousedown]',
    '[onmouseup]',
    '[onkeydown]',
    '[onkeyup]',
    '[tabindex]',
    '[contenteditable]:not([contenteditable="false"])',
    '[aria-checked]',
    '[aria-expanded]',
    '[aria-pressed]',
    '[aria-selected]'
  ].join(',');
  const hasFormControlDescendant = (el, depth) => {{
    if (depth <= 0) return false;
    for (const child of Array.from(el.children || [])) {{
      const tag = child.tagName ? child.tagName.toLowerCase() : '';
      if (['input', 'select', 'textarea'].includes(tag)) return true;
      if (hasFormControlDescendant(child, depth - 1)) return true;
    }}
    return false;
  }};
  const hasSearchIndicator = (el) => {{
    const indicators = ['search', 'magnify', 'glass', 'lookup', 'find', 'query', 'search-icon', 'search-btn', 'search-button', 'searchbox'];
    const classText = String(el.getAttribute('class') || '').toLowerCase();
    const idText = String(el.getAttribute('id') || '').toLowerCase();
    if (indicators.some((indicator) => classText.includes(indicator) || idText.includes(indicator))) return true;
    for (const attr of Array.from(el.attributes || [])) {{
      if (attr.name.startsWith('data-') && indicators.some((indicator) => String(attr.value || '').toLowerCase().includes(indicator))) return true;
    }}
    return false;
  }};
  const hasAriaInteractivityProperty = (el) => {{
    const required = String(el.getAttribute('aria-required') || '').toLowerCase();
    if (required === 'true') return true;
    const autocomplete = String(el.getAttribute('aria-autocomplete') || '').toLowerCase();
    if (autocomplete && autocomplete !== 'none') return true;
    return String(el.getAttribute('aria-keyshortcuts') || '').trim().length > 0;
  }};
  const hasIconSignal = (el) => {{
    const rect = el.getBoundingClientRect();
    if (rect.width < 10 || rect.width > 50 || rect.height < 10 || rect.height > 50) return false;
    return ['class', 'role', 'onclick', 'data-action', 'aria-label'].some((name) => el.hasAttribute(name));
  }};
  const hasPointerCursor = (el) => {{
    try {{
      return window.getComputedStyle(el).cursor === 'pointer';
    }} catch (_) {{
      return false;
    }}
  }};
  const isBrowserUseExcluded = (el) => {{
    const legacy = el.getAttribute('data-browser-use-exclude');
    if (typeof legacy === 'string' && legacy.toLowerCase() === 'true') return true;
    for (const attr of Array.from(el.attributes || [])) {{
      if (attr.name.startsWith('data-browser-use-exclude-') && String(attr.value || '').toLowerCase() === 'true') return true;
    }}
    return false;
  }};
  const isFileInput = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return tag === 'input' && (el.getAttribute('type') || '').toLowerCase() === 'file';
  }};
  const isTopmostAtCenter = (el) => {{
    if (isFileInput(el)) return true;
    try {{
      const rect = el.getBoundingClientRect();
      const doc = el.ownerDocument || document;
      const view = doc.defaultView || window;
      const x = rect.left + rect.width / 2;
      const y = rect.top + rect.height / 2;
      if (x < 0 || y < 0 || x >= view.innerWidth || y >= view.innerHeight) return true;
      const top = doc.elementFromPoint(x, y);
      if (!top) return true;
      if (top === el || el.contains(top)) return true;
      const root = el.getRootNode && el.getRootNode();
      return Boolean(root && root.host && (top === root.host || root.host.contains(top)));
    }} catch (_) {{
      return true;
    }}
  }};
  const isDecorativeSvgChild = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return ['path', 'rect', 'g', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'use', 'defs', 'clippath', 'mask', 'pattern', 'image', 'text', 'tspan'].includes(tag);
  }};
  const isNonContentTag = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return ['style', 'script', 'head', 'meta', 'link', 'title'].includes(tag);
  }};
  const isPropagatingActionContainer = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const role = String(el.getAttribute('role') || '').toLowerCase();
    return tag === 'a'
      || tag === 'button'
      || ((tag === 'div' || tag === 'span') && (role === 'button' || role === 'combobox'))
      || (tag === 'input' && role === 'combobox');
  }};
  const shouldKeepContainedDescendant = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const role = String(el.getAttribute('role') || '').toLowerCase();
    if (['input', 'select', 'textarea', 'label'].includes(tag)) return true;
    if (isPropagatingActionContainer(el)) return true;
    if (el.hasAttribute('onclick')) return true;
    if (String(el.getAttribute('aria-label') || '').trim()) return true;
    return ['button', 'link', 'checkbox', 'radio', 'tab', 'menuitem', 'option'].includes(role);
  }};
  const parentElementOrShadowHost = (node) => {{
    if (node.parentElement) return node.parentElement;
    const root = node.getRootNode?.();
    return root?.host instanceof Element ? root.host : null;
  }};
  const containedByRect = (childRect, parentRect) => {{
    const childArea = childRect.width * childRect.height;
    if (childArea <= 0) return false;
    const xOverlap = Math.max(0, Math.min(childRect.x + childRect.width, parentRect.x + parentRect.width) - Math.max(childRect.x, parentRect.x));
    const yOverlap = Math.max(0, Math.min(childRect.y + childRect.height, parentRect.y + parentRect.height) - Math.max(childRect.y, parentRect.y));
    return (xOverlap * yOverlap) / childArea >= 0.99;
  }};
  const isContainedByPropagatingActionContainer = (el) => {{
    if (shouldKeepContainedDescendant(el)) return false;
    const rect = el.getBoundingClientRect();
    let ancestor = parentElementOrShadowHost(el);
    while (ancestor) {{
      if (isPropagatingActionContainer(ancestor) && isVisible(ancestor) && containedByRect(rect, ancestor.getBoundingClientRect())) return true;
      ancestor = parentElementOrShadowHost(ancestor);
    }}
    return false;
  }};
  const isDisabledOrHidden = (el) => {{
    return isBrowserUseExcluded(el) || el.hidden || el.disabled === true || el.getAttribute('aria-hidden') === 'true' || el.getAttribute('aria-disabled') === 'true';
  }};
  const isVisible = (el) => {{
    if (isBrowserUseExcluded(el) || el.disabled === true || el.getAttribute('aria-hidden') === 'true' || el.getAttribute('aria-disabled') === 'true') return false;
    if (isFileInput(el)) return true;
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return !isDisabledOrHidden(el) && rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden' && isTopmostAtCenter(el);
  }};
  const isScrollable = (el) => {{
    const style = window.getComputedStyle(el);
    const overflow = `${{style.overflow}} ${{style.overflowX}} ${{style.overflowY}}`;
    return /(auto|scroll|overlay)/.test(overflow) && (el.scrollHeight > el.clientHeight || el.scrollWidth > el.clientWidth);
  }};
  const isDropdownContainer = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classText = String(el.getAttribute('class') || '').toLowerCase();
    const classes = classText.split(/\s+/).filter(Boolean);
    return tag === 'select'
      || ['listbox', 'menu', 'combobox', 'menubar', 'tree', 'grid'].includes(role)
      || classes.includes('dropdown')
      || classes.includes('dropdown-menu')
      || classes.includes('select-menu')
      || (classes.includes('ui') && classText.includes('dropdown'));
  }};
  const isInteractive = (el) => {{
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag === 'html' || tag === 'body') return false;
    if (isNonContentTag(el)) return false;
    if (tag === 'iframe' || tag === 'frame') {{
      const rect = el.getBoundingClientRect();
      return rect.width > 100 && rect.height > 100;
    }}
    if (tag === 'label') return !el.hasAttribute('for') && hasFormControlDescendant(el, 2);
    if (tag === 'span' && hasFormControlDescendant(el, 2)) return true;
    if (hasAriaInteractivityProperty(el)) return true;
    if (hasSearchIndicator(el)) return true;
    if (hasIconSignal(el)) return true;
    if (hasPointerCursor(el)) return true;
    return el.matches(selector);
  }};
  const hasInteractiveDescendant = (el) => {{
    const visit = (root) => {{
      for (const child of Array.from(root.children || [])) {{
        if (isDecorativeSvgChild(child) || isNonContentTag(child) || isBrowserUseExcluded(child)) continue;
        if (isInteractive(child) && isVisible(child)) return true;
        if (child.shadowRoot && visit(child.shadowRoot)) return true;
        if (visit(child)) return true;
      }}
      return false;
    }};
    return visit(el);
  }};
  const shouldIndexScrollable = (el) => {{
    return isScrollable(el) && (isDropdownContainer(el) || !hasInteractiveDescendant(el));
  }};
  const elements = [];
  const visitFrame = (iframe, offset) => {{
    if (!isVisible(iframe)) return;
    try {{
      const frameDocument = iframe.contentDocument;
      if (!frameDocument) return;
      const rect = iframe.getBoundingClientRect();
      visitChildren(frameDocument, {{ x: offset.x + rect.x, y: offset.y + rect.y }});
    }} catch (_) {{
      return;
    }}
  }};
  const visitNode = (node, offset) => {{
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    if (isDecorativeSvgChild(node)) return;
    if (isNonContentTag(node)) return;
    if (isBrowserUseExcluded(node)) return;
    if ((isInteractive(node) || shouldIndexScrollable(node)) && isVisible(node) && !isContainedByPropagatingActionContainer(node)) elements.push(node);
    if (node.shadowRoot) visitChildren(node.shadowRoot, offset);
    if (node.tagName && node.tagName.toLowerCase() === 'iframe') visitFrame(node, offset);
    visitChildren(node, offset);
  }};
  const visitChildren = (root, offset) => {{
    for (const child of Array.from(root.children || [])) visitNode(child, offset);
  }};
  visitChildren(document, {{ x: 0, y: 0 }});
  const el = elements[{zero_based}];
  if (!el) throw new Error('No interactive element found for index {index}');
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  {body}
}})()
"#,
        zero_based = index.saturating_sub(1),
        index = index,
        body = body
    )
}

pub(crate) fn element_action_js(index: u32, action: &str) -> String {
    element_eval_js(index, &format!("{action}\n  return true;"))
}

pub(crate) fn element_function_js(body: &str) -> String {
    format!(
        r#"function() {{
  const el = this;
  if (!el.isConnected) {{
    throw new Error('cached element is detached from DOM');
  }}
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  {body}
}}"#
    )
}

pub(crate) fn element_action_function_js(action: &str) -> String {
    element_function_js(&format!("{action}\n  return true;"))
}

pub(crate) fn interaction_highlight_duration_ms(duration_seconds: f64) -> u64 {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0;
    }
    (duration_seconds.min(86_400.0) * 1000.0).round() as u64
}

pub(crate) fn interaction_element_highlight_script(
    bounds: ElementBounds,
    color: &str,
    duration_seconds: f64,
) -> String {
    let rect = json!({
        "x": bounds.x,
        "y": bounds.y,
        "width": bounds.width,
        "height": bounds.height,
    });
    let color = serde_json::to_string(color).unwrap_or_else(|_| "\"rgb(255, 127, 39)\"".to_owned());
    let duration_ms = interaction_highlight_duration_ms(duration_seconds);
    format!(
        r#"(function() {{
  const rect = {rect};
  if (!rect || rect.width <= 0 || rect.height <= 0) return true;
  const color = {color};
  const duration = {duration_ms};
  const scrollX = window.pageXOffset || document.documentElement.scrollLeft || 0;
  const scrollY = window.pageYOffset || document.documentElement.scrollTop || 0;
  const container = document.createElement('div');
  container.setAttribute('data-browser-use-interaction-highlight', 'true');
  container.style.cssText = `
    position: absolute;
    left: ${{rect.x + scrollX}}px;
    top: ${{rect.y + scrollY}}px;
    width: ${{rect.width}}px;
    height: ${{rect.height}}px;
    pointer-events: none;
    z-index: 2147483647;
  `;
  const cornerSize = Math.max(8, Math.min(20, Math.min(rect.width, rect.height) * 0.35));
  const borderWidth = 3;
  const corners = [
    ['top-left', 'top', 'left', 'borderTop', 'borderLeft'],
    ['top-right', 'top', 'right', 'borderTop', 'borderRight'],
    ['bottom-left', 'bottom', 'left', 'borderBottom', 'borderLeft'],
    ['bottom-right', 'bottom', 'right', 'borderBottom', 'borderRight'],
  ];
  for (const corner of corners) {{
    const bracket = document.createElement('div');
    bracket.setAttribute('data-browser-use-interaction-highlight', corner[0]);
    bracket.style.cssText = `
      position: absolute;
      width: ${{cornerSize}}px;
      height: ${{cornerSize}}px;
      pointer-events: none;
      opacity: 0.92;
      transition: opacity 0.3s ease-out;
    `;
    bracket.style[corner[1]] = '-3px';
    bracket.style[corner[2]] = '-3px';
    bracket.style[corner[3]] = `${{borderWidth}}px solid ${{color}}`;
    bracket.style[corner[4]] = `${{borderWidth}}px solid ${{color}}`;
    container.appendChild(bracket);
  }}
  document.body.appendChild(container);
  setTimeout(() => {{
    container.style.opacity = '0';
    container.style.transition = 'opacity 0.3s ease-out';
    setTimeout(() => container.remove(), 300);
  }}, duration);
  return true;
}})()"#
    )
}

pub(crate) fn interaction_coordinate_highlight_script(
    x: i32,
    y: i32,
    color: &str,
    duration_seconds: f64,
) -> String {
    let color = serde_json::to_string(color).unwrap_or_else(|_| "\"rgb(255, 127, 39)\"".to_owned());
    let duration_ms = interaction_highlight_duration_ms(duration_seconds);
    format!(
        r#"(function() {{
  const x = {x};
  const y = {y};
  const color = {color};
  const duration = {duration_ms};
  const scrollX = window.pageXOffset || document.documentElement.scrollLeft || 0;
  const scrollY = window.pageYOffset || document.documentElement.scrollTop || 0;
  const container = document.createElement('div');
  container.setAttribute('data-browser-use-coordinate-highlight', 'true');
  container.style.cssText = `
    position: absolute;
    left: ${{x + scrollX}}px;
    top: ${{y + scrollY}}px;
    width: 0;
    height: 0;
    pointer-events: none;
    z-index: 2147483647;
  `;
  const ring = document.createElement('div');
  ring.style.cssText = `
    position: absolute;
    left: -15px;
    top: -15px;
    width: 30px;
    height: 30px;
    border: 3px solid ${{color}};
    border-radius: 50%;
    opacity: 0.85;
  `;
  const dot = document.createElement('div');
  dot.style.cssText = `
    position: absolute;
    left: -4px;
    top: -4px;
    width: 8px;
    height: 8px;
    background: ${{color}};
    border-radius: 50%;
  `;
  container.appendChild(ring);
  container.appendChild(dot);
  document.body.appendChild(container);
  setTimeout(() => {{
    container.style.opacity = '0';
    container.style.transition = 'opacity 0.3s ease-out';
    setTimeout(() => container.remove(), 300);
  }}, duration);
  return true;
}})()"#
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DomHighlightOverlayElement {
    pub(crate) index: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
}

pub(crate) fn dom_highlight_overlay_elements(
    selector_map: &BTreeMap<u32, DomElementRef>,
    filter_highlight_ids: bool,
) -> Vec<DomHighlightOverlayElement> {
    selector_map
        .iter()
        .filter_map(|(index, element)| {
            let bounds = element.bounds?;
            if bounds.width == 0 || bounds.height == 0 {
                return None;
            }
            let representation = render_element_text(element);
            let label = (!filter_highlight_ids || representation.chars().count() < 10)
                .then(|| index.to_string());
            Some(DomHighlightOverlayElement {
                index: *index,
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
                label,
            })
        })
        .collect()
}

pub(crate) fn dom_highlight_overlay_script(elements: &[DomHighlightOverlayElement]) -> String {
    let elements = serde_json::to_string(elements).unwrap_or_else(|_| "[]".to_owned());
    format!(
        r#"(function() {{
  const elements = {elements};
  document.querySelectorAll('[data-browser-use-highlight]').forEach((element) => element.remove());
  const oldContainer = document.getElementById('browser-use-debug-highlights');
  if (oldContainer) oldContainer.remove();
  const container = document.createElement('div');
  container.id = 'browser-use-debug-highlights';
  container.setAttribute('data-browser-use-highlight', 'container');
  container.style.cssText = `
    position: absolute;
    left: 0;
    top: 0;
    width: 0;
    height: 0;
    pointer-events: none;
    z-index: 2147483646;
  `;
  const scrollX = window.pageXOffset || document.documentElement.scrollLeft || 0;
  const scrollY = window.pageYOffset || document.documentElement.scrollTop || 0;
  for (const element of elements) {{
    const box = document.createElement('div');
    box.setAttribute('data-browser-use-highlight', 'box');
    box.setAttribute('data-browser-use-index', String(element.index));
    box.style.cssText = `
      position: absolute;
      left: ${{element.x + scrollX}}px;
      top: ${{element.y + scrollY}}px;
      width: ${{element.width}}px;
      height: ${{element.height}}px;
      border: 2px solid rgba(255, 127, 39, 0.92);
      background: rgba(255, 127, 39, 0.10);
      box-sizing: border-box;
      pointer-events: none;
    `;
    if (element.label) {{
      const label = document.createElement('div');
      label.setAttribute('data-browser-use-highlight', 'tooltip');
      label.textContent = element.label;
      label.style.cssText = `
        position: absolute;
        left: ${{element.x + scrollX}}px;
        top: ${{Math.max(0, element.y + scrollY - 18)}}px;
        min-width: 16px;
        height: 16px;
        padding: 0 4px;
        border-radius: 2px;
        background: rgb(255, 127, 39);
        color: #111;
        font: 12px/16px monospace;
        text-align: center;
        pointer-events: none;
      `;
      container.appendChild(label);
    }}
    container.appendChild(box);
  }}
  document.body.appendChild(container);
  return true;
}})()"#
    )
}

pub(crate) const CLICK_ELEMENT_ACTION_JS: &str = r#"const tag = el.tagName ? el.tagName.toLowerCase() : '';
  if (tag === 'select') {
    throw new Error('Cannot click on <select> elements. Use get_dropdown_options and select_dropdown_option instead.');
  }
  if (tag === 'input' && (el.getAttribute('type') || '').toLowerCase() === 'file') {
    throw new Error('Cannot click on file input elements. Use upload_file instead.');
  }
  if (typeof el.focus === 'function') el.focus();
  if (typeof el.click === 'function') {
    el.click();
  } else {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }));
  }"#;

pub(crate) fn click_element_js(index: u32) -> String {
    element_action_js(index, CLICK_ELEMENT_ACTION_JS)
}

pub(crate) fn dropdown_options_js(index: u32) -> String {
    element_eval_js(index, DROPDOWN_OPTIONS_BODY_JS)
}

pub(crate) const DROPDOWN_OPTIONS_BODY_JS: &str = r#"
  const textOf = (node) => (node.innerText || node.textContent || node.getAttribute('aria-label') || node.getAttribute('value') || '').trim();
  const isVisible = (node) => {
    const style = window.getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && rect.width >= 0 && rect.height >= 0;
  };
  const addOption = (seen, node) => {
    if (!node || seen.has(node) || !isVisible(node)) return;
    const text = textOf(node);
    if (text) seen.set(node, text);
  };
  const collectOptions = (seen, root) => {
    if (!root || !root.querySelectorAll) return;
    for (const node of root.querySelectorAll('option, [role="option"], [role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], [data-value]')) {
      addOption(seen, node);
    }
  };
  if (el.tagName.toLowerCase() === 'select') {
    return JSON.stringify(Array.from(el.options).map((option) => (option.text || option.value || '').trim()).filter(Boolean));
  }
  const seen = new Map();
  collectOptions(seen, el);
  for (const attr of ['aria-controls', 'aria-owns']) {
    for (const id of (el.getAttribute(attr) || '').split(/\s+/).filter(Boolean)) {
      collectOptions(seen, el.ownerDocument.getElementById(id));
    }
  }
  const options = Array.from(seen.values());
  if (options.length === 0) {
    throw new Error('Element is not a select, ARIA listbox, combobox, or menu with visible options');
  }
  return JSON.stringify(options);
"#;

pub(crate) fn select_dropdown_option_body_js(text: &str) -> Result<String, BrowserError> {
    let text_json =
        serde_json::to_string(text).map_err(|error| BrowserError::Transport(error.to_string()))?;
    Ok(format!(
        r#"
  const requested = {text_json};
  const textOf = (node) => (node.innerText || node.textContent || node.getAttribute('aria-label') || node.getAttribute('value') || '').trim();
  const isVisible = (node) => {{
    const style = window.getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && rect.width >= 0 && rect.height >= 0;
  }};
  const matchesRequested = (node) => {{
    return node.getAttribute('value') === requested || textOf(node) === requested;
  }};
  if (el.tagName.toLowerCase() === 'select') {{
    const option = Array.from(el.options).find(matchesRequested);
    if (!option) throw new Error(`No dropdown option found for ${{requested}}`);
    el.value = option.value;
    option.selected = true;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return true;
  }}
  const candidates = [];
  const collectOptions = (root) => {{
    if (!root || !root.querySelectorAll) return;
    for (const node of root.querySelectorAll('option, [role="option"], [role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], [data-value]')) {{
      if (isVisible(node)) candidates.push(node);
    }}
  }};
  collectOptions(el);
  for (const attr of ['aria-controls', 'aria-owns']) {{
    for (const id of (el.getAttribute(attr) || '').split(/\s+/).filter(Boolean)) {{
      collectOptions(el.ownerDocument.getElementById(id));
    }}
  }}
  const option = candidates.find(matchesRequested);
  if (!option) throw new Error(`No dropdown option found for ${{requested}}`);
  option.setAttribute('aria-selected', 'true');
  option.click();
  option.dispatchEvent(new MouseEvent('click', {{ bubbles: true, cancelable: true, view: window }}));
  option.dispatchEvent(new Event('input', {{ bubbles: true }}));
  option.dispatchEvent(new Event('change', {{ bubbles: true }}));
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return true;
"#
    ))
}

pub(crate) fn select_dropdown_option_js(index: u32, text: &str) -> Result<String, BrowserError> {
    Ok(element_eval_js(
        index,
        &select_dropdown_option_body_js(text)?,
    ))
}

pub(crate) fn scroll_to_text_js(text: &str) -> Result<String, BrowserError> {
    let text =
        serde_json::to_string(text).map_err(|error| BrowserError::Transport(error.to_string()))?;
    Ok(format!(
        r#"(() => {{
  const needle = {text};
  const root = document.body || document.documentElement;
  if (!root || !needle) return false;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {{
    acceptNode(node) {{
      if (!node.textContent || !node.textContent.includes(needle)) return NodeFilter.FILTER_REJECT;
      const parent = node.parentElement;
      if (!parent) return NodeFilter.FILTER_REJECT;
      const style = window.getComputedStyle(parent);
      const rect = parent.getBoundingClientRect();
      if (style.display === 'none' || style.visibility === 'hidden') return NodeFilter.FILTER_REJECT;
      if (rect.width === 0 && rect.height === 0) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    }}
  }});
  const node = walker.nextNode();
  if (!node || !node.parentElement) return false;
  node.parentElement.scrollIntoView({{ behavior: 'instant', block: 'center', inline: 'nearest' }});
  return true;
}})()"#
    ))
}
