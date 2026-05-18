//! Chrome DevTools Protocol browser-session layer.

use std::collections::BTreeMap;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use browser_use_dom::{
    BrowserStateSummary, DomElementRef, DomPageStats, ElementBounds, PageInfo, PaginationButton,
    PaginationButtonType, SerializedDomState, TabInfo, render_element_text,
};
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const AX_REF_ATTRIBUTE: &str = "data-browser-use-rs-ax-ref";
const URL_POLICY_SETTLE_MS: u64 = 200;

const INTERACTIVE_ELEMENTS_JS: &str = r#"
(() => {
  const axRefAttribute = 'data-browser-use-rs-ax-ref';
  const selector = [
    'a[href]',
    'button',
    'input',
    'textarea',
    'select',
    'details',
    'summary',
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
    '[tabindex]:not([tabindex="-1"])',
    '[contenteditable="true"]',
    '[contenteditable=""]',
    '[aria-checked]',
    '[aria-expanded]',
    '[aria-pressed]',
    '[aria-selected]'
  ].join(',');
  const hasFormControlDescendant = (el, depth) => {
    if (depth <= 0) return false;
    for (const child of Array.from(el.children || [])) {
      const tag = child.tagName ? child.tagName.toLowerCase() : '';
      if (['input', 'select', 'textarea'].includes(tag)) return true;
      if (hasFormControlDescendant(child, depth - 1)) return true;
    }
    return false;
  };
  const hasSearchIndicator = (el) => {
    const indicators = ['search', 'magnify', 'glass', 'lookup', 'find', 'query', 'search-icon', 'search-btn', 'search-button', 'searchbox'];
    const classText = String(el.getAttribute('class') || '').toLowerCase();
    const idText = String(el.getAttribute('id') || '').toLowerCase();
    if (indicators.some((indicator) => classText.includes(indicator) || idText.includes(indicator))) return true;
    for (const attr of Array.from(el.attributes || [])) {
      if (attr.name.startsWith('data-') && indicators.some((indicator) => String(attr.value || '').toLowerCase().includes(indicator))) return true;
    }
    return false;
  };
  const hasIconSignal = (el) => {
    const rect = el.getBoundingClientRect();
    if (rect.width < 10 || rect.width > 50 || rect.height < 10 || rect.height > 50) return false;
    return ['class', 'role', 'onclick', 'data-action', 'aria-label'].some((name) => el.hasAttribute(name));
  };
  const hasPointerCursor = (el) => {
    try {
      return window.getComputedStyle(el).cursor === 'pointer';
    } catch (_) {
      return false;
    }
  };
  const canInspectJsListeners = typeof getEventListeners === 'function' && document.querySelectorAll('*').length <= 10000;
  const hasJsClickListener = (el) => {
    if (!canInspectJsListeners) return false;
    try {
      const listeners = getEventListeners(el) || {};
      return ['click', 'mousedown', 'mouseup', 'pointerdown', 'pointerup'].some((type) => Array.isArray(listeners[type]) && listeners[type].length > 0);
    } catch (_) {
      return false;
    }
  };
  const isBrowserUseExcluded = (el) => {
    const legacy = el.getAttribute('data-browser-use-exclude');
    if (typeof legacy === 'string' && legacy.toLowerCase() === 'true') return true;
    for (const attr of Array.from(el.attributes || [])) {
      if (attr.name.startsWith('data-browser-use-exclude-') && String(attr.value || '').toLowerCase() === 'true') return true;
    }
    return false;
  };
  const isFileInput = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return tag === 'input' && (el.getAttribute('type') || '').toLowerCase() === 'file';
  };
  const isTopmostAtCenter = (el) => {
    if (isFileInput(el)) return true;
    try {
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
    } catch (_) {
      return true;
    }
  };
  const isDecorativeSvgChild = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return ['path', 'rect', 'g', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'use', 'defs', 'clippath', 'mask', 'pattern', 'image', 'text', 'tspan'].includes(tag);
  };
  const isNonContentTag = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    return ['style', 'script', 'head', 'meta', 'link', 'title'].includes(tag);
  };
  const isPropagatingActionContainer = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const role = String(el.getAttribute('role') || '').toLowerCase();
    return tag === 'a'
      || tag === 'button'
      || ((tag === 'div' || tag === 'span') && (role === 'button' || role === 'combobox'))
      || (tag === 'input' && role === 'combobox');
  };
  const shouldKeepContainedDescendant = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const role = String(el.getAttribute('role') || '').toLowerCase();
    if (['input', 'select', 'textarea', 'label'].includes(tag)) return true;
    if (isPropagatingActionContainer(el)) return true;
    if (el.hasAttribute('onclick')) return true;
    if (String(el.getAttribute('aria-label') || '').trim()) return true;
    return ['button', 'link', 'checkbox', 'radio', 'tab', 'menuitem', 'option'].includes(role);
  };
  const parentElementOrShadowHost = (node) => {
    if (node.parentElement) return node.parentElement;
    const root = node.getRootNode?.();
    return root?.host instanceof Element ? root.host : null;
  };
  const containedByRect = (childRect, parentRect) => {
    const childArea = childRect.width * childRect.height;
    if (childArea <= 0) return false;
    const xOverlap = Math.max(0, Math.min(childRect.x + childRect.width, parentRect.x + parentRect.width) - Math.max(childRect.x, parentRect.x));
    const yOverlap = Math.max(0, Math.min(childRect.y + childRect.height, parentRect.y + parentRect.height) - Math.max(childRect.y, parentRect.y));
    return (xOverlap * yOverlap) / childArea >= 0.99;
  };
  const isContainedByPropagatingActionContainer = (el) => {
    if (shouldKeepContainedDescendant(el)) return false;
    const rect = el.getBoundingClientRect();
    let ancestor = parentElementOrShadowHost(el);
    while (ancestor) {
      if (isPropagatingActionContainer(ancestor) && isVisible(ancestor) && containedByRect(rect, ancestor.getBoundingClientRect())) return true;
      ancestor = parentElementOrShadowHost(ancestor);
    }
    return false;
  };
  const isDisabledOrHidden = (el) => {
    return isBrowserUseExcluded(el) || el.hidden || el.disabled === true || el.getAttribute('aria-hidden') === 'true' || el.getAttribute('aria-disabled') === 'true';
  };
  const isVisible = (el) => {
    if (isBrowserUseExcluded(el) || el.disabled === true || el.getAttribute('aria-hidden') === 'true' || el.getAttribute('aria-disabled') === 'true') return false;
    if (isFileInput(el)) return true;
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return !isDisabledOrHidden(el) && rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden' && isTopmostAtCenter(el);
  };
  const isInteractive = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag === 'html' || tag === 'body') return false;
    if (isNonContentTag(el)) return false;
    if (tag === 'iframe' || tag === 'frame') {
      const rect = el.getBoundingClientRect();
      return rect.width > 100 && rect.height > 100;
    }
    if (hasJsClickListener(el)) return true;
    if (tag === 'label') return !el.hasAttribute('for') && hasFormControlDescendant(el, 2);
    if (tag === 'span' && hasFormControlDescendant(el, 2)) return true;
    if (hasSearchIndicator(el)) return true;
    if (hasIconSignal(el)) return true;
    if (hasPointerCursor(el)) return true;
    return el.matches(selector);
  };
  const isScrollable = (el) => {
    const style = window.getComputedStyle(el);
    const overflow = `${style.overflow} ${style.overflowX} ${style.overflowY}`;
    return /(auto|scroll|overlay)/.test(overflow) && (el.scrollHeight > el.clientHeight || el.scrollWidth > el.clientWidth);
  };
  const isDropdownContainer = (el) => {
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
  };
  const hasInteractiveDescendant = (el) => {
    const visit = (root) => {
      for (const child of Array.from(root.children || [])) {
        if (isDecorativeSvgChild(child) || isNonContentTag(child) || isBrowserUseExcluded(child)) continue;
        if (isInteractive(child) && isVisible(child)) return true;
        if (child.shadowRoot && visit(child.shadowRoot)) return true;
        if (visit(child)) return true;
      }
      return false;
    };
    return visit(el);
  };
  const shouldIndexScrollable = (el) => {
    return isScrollable(el) && (isDropdownContainer(el) || !hasInteractiveDescendant(el));
  };
  const referencedText = (el, attribute) => {
    const ids = (el.getAttribute(attribute) || '').split(/\s+/).filter(Boolean);
    return ids.map((id) => {
      const ref = el.ownerDocument.getElementById(id);
      return ref ? (ref.innerText || ref.textContent || '').trim() : '';
    }).filter(Boolean).join(' ');
  };
  const labelText = (el) => {
    const aria = referencedText(el, 'aria-labelledby');
    if (aria) return aria;
    const labels = Array.from(el.labels || []).map((label) => (label.innerText || label.textContent || '').trim()).filter(Boolean);
    return labels.join(' ');
  };
  const descendantAltText = (el) => {
    return Array.from(el.querySelectorAll?.('img[alt], svg[aria-label]') || [])
      .map((child) => child.getAttribute('alt') || child.getAttribute('aria-label') || '')
      .map((value) => value.trim())
      .filter(Boolean)
      .join(' ');
  };
  const controlValueText = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag === 'select') {
      return Array.from(el.selectedOptions || [])
        .map((option) => (option.text || option.value || '').trim())
        .filter(Boolean)
        .join(' ');
    }
    if (tag === 'input' || tag === 'textarea') return (el.value || '').trim();
    return '';
  };
  const compactOptionText = (value) => {
    const text = String(value || '').replace(/\s+/g, ' ').trim();
    return text.length > 30 ? `${text.slice(0, 30)}...` : text;
  };
  const inferSelectFormatHint = (values) => {
    const sample = values.filter(Boolean).slice(0, 5);
    if (sample.length < 2) return '';
    if (sample.every((value) => /^\d+$/.test(value))) return 'numeric';
    if (sample.every((value) => value.length === 2 && value === value.toUpperCase())) return 'country/state codes';
    if (sample.every((value) => value.includes('/') || value.includes('-'))) return 'date/path format';
    if (sample.some((value) => value.includes('@'))) return 'email addresses';
    return '';
  };
  const selectCompoundComponents = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag !== 'select') return '';
    const options = Array.from(el.querySelectorAll('option') || [])
      .map((option) => {
        const text = compactOptionText(option.text || option.textContent || option.value || '');
        const value = String(option.getAttribute('value') || option.value || text || '').trim();
        return { text, value };
      })
      .filter((option) => option.text || option.value);
    const components = ['(name=Dropdown Toggle,role=button)'];
    const optionParts = ['name=Options', 'role=listbox'];
    if (options.length > 0) {
      optionParts.push(`count=${options.length}`);
      const firstOptions = options.slice(0, 4).map((option) => option.text || compactOptionText(option.value)).filter(Boolean);
      if (options.length > 4) firstOptions.push(`... ${options.length - 4} more options...`);
      if (firstOptions.length > 0) optionParts.push(`options=${firstOptions.join('|')}`);
      const formatHint = inferSelectFormatHint(options.map((option) => option.value));
      if (formatHint) optionParts.push(`format=${formatHint}`);
    }
    components.push(`(${optionParts.join(',')})`);
    return components.join(',');
  };
  const elements = [];
  const stats = {
    links: 0,
    iframes: 0,
    shadow_open: 0,
    shadow_closed: 0,
    scroll_containers: 0,
    images: 0,
    interactive_elements: 0,
    total_elements: 0,
    text_chars: 0
  };
  const addDirectTextStats = (node) => {
    for (const child of Array.from(node.childNodes || [])) {
      if (child.nodeType === Node.TEXT_NODE) stats.text_chars += (child.nodeValue || '').trim().length;
    }
  };
  const visitFrame = (iframe, offset) => {
    if (!isVisible(iframe)) return;
    try {
      const frameDocument = iframe.contentDocument;
      if (!frameDocument) return;
      const rect = iframe.getBoundingClientRect();
      visitChildren(frameDocument, { x: offset.x + rect.x, y: offset.y + rect.y });
    } catch (_) {
      return;
    }
  };
  const visitNode = (node, offset) => {
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    if (isDecorativeSvgChild(node)) return;
    if (isNonContentTag(node)) return;
    if (isBrowserUseExcluded(node)) return;
    stats.total_elements += 1;
    addDirectTextStats(node);
    const tag = node.tagName ? node.tagName.toLowerCase() : '';
    if (tag === 'a') stats.links += 1;
    if (tag === 'iframe' || tag === 'frame') stats.iframes += 1;
    if (tag === 'img') stats.images += 1;
    if (isScrollable(node)) stats.scroll_containers += 1;
    const interactive = (isInteractive(node) || shouldIndexScrollable(node)) && isVisible(node) && !isContainedByPropagatingActionContainer(node);
    if (interactive) {
      stats.interactive_elements += 1;
      elements.push({ el: node, offset });
    }
    if (node.shadowRoot) {
      stats.shadow_open += 1;
      visitChildren(node.shadowRoot, offset);
    }
    if (tag === 'iframe' || tag === 'frame') visitFrame(node, offset);
    visitChildren(node, offset);
  };
  const visitChildren = (root, offset) => {
    for (const child of Array.from(root.children || [])) visitNode(child, offset);
  };
  visitChildren(document, { x: 0, y: 0 });
  return {
    stats,
    elements: elements.slice(0, 400).map(({ el, offset }, index) => {
    const rect = el.getBoundingClientRect();
    const axRef = `browser-use-rs-${index + 1}`;
    try { el.setAttribute(axRefAttribute, axRef); } catch (_) {}
    const attrs = {};
    for (const name of ['id', 'class', 'name', 'type', 'placeholder', 'value', 'href', 'src', 'alt', 'aria-label', 'aria-labelledby', 'aria-describedby', 'aria-atomic', 'aria-autocomplete', 'aria-busy', 'aria-checked', 'aria-controls', 'aria-current', 'aria-disabled', 'aria-expanded', 'aria-haspopup', 'aria-hidden', 'aria-invalid', 'aria-live', 'aria-owns', 'aria-placeholder', 'aria-pressed', 'aria-required', 'aria-selected', 'aria-valuemax', 'aria-valuemin', 'aria-valuenow', 'role', 'title', 'contenteditable', 'data-cy', 'data-selenium', 'data-test', 'data-testid', 'data-qa', 'data-state', 'data-value', 'data-mask', 'data-inputmask', 'data-datepicker', 'data-date-format', 'uib-datepicker-popup', 'for', 'required', 'disabled', 'readonly', 'selected', 'multiple', 'accept', 'target', 'rel', 'list', 'tabindex', 'inputmode', 'autocomplete', 'pattern', 'min', 'max', 'minlength', 'maxlength', 'step', 'lang', 'itemscope', 'itemtype', 'itemprop', 'pseudo']) {
      const value = el.getAttribute(name);
      if (value) attrs[name] = value;
    }
    const altText = descendantAltText(el);
    const controlText = controlValueText(el);
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (controlText && type !== 'password') attrs.value = controlText;
    if ((tag === 'input' || tag === 'option') && 'checked' in el) attrs.checked = String(el.checked);
    if (tag === 'option' && 'selected' in el) attrs.selected = String(el.selected);
    const compoundComponents = selectCompoundComponents(el);
    if (compoundComponents) attrs.compound_components = compoundComponents;
    const text = (controlText || el.innerText || altText || '').trim().slice(0, 200);
    const name = (el.getAttribute('aria-label') || labelText(el) || el.getAttribute('title') || el.getAttribute('placeholder') || el.getAttribute('alt') || referencedText(el, 'aria-describedby') || altText || text || '').trim();
    return {
      index: index + 1,
      tag_name: el.tagName.toLowerCase(),
      role: el.getAttribute('role'),
      name,
      text,
      attributes: attrs,
      bounds: {
        x: Math.round(rect.x + offset.x),
        y: Math.round(rect.y + offset.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height)
      },
      is_visible: true,
      is_interactive: true,
      is_scrollable: isScrollable(el),
      ax_ref: axRef
    };
  })};
})()
"#;

const CLEANUP_AX_REFS_JS: &str = r#"
(() => {
  const attr = 'data-browser-use-rs-ax-ref';
  const cleanRoot = (root) => {
    for (const el of Array.from(root.querySelectorAll?.(`[${attr}]`) || [])) {
      el.removeAttribute(attr);
    }
    for (const el of Array.from(root.querySelectorAll?.('*') || [])) {
      if (el.shadowRoot) cleanRoot(el.shadowRoot);
      const tag = el.tagName ? el.tagName.toLowerCase() : '';
      if (tag === 'iframe' || tag === 'frame') {
        try {
          if (el.contentDocument) cleanRoot(el.contentDocument);
        } catch (_) {}
      }
    }
  };
  cleanRoot(document);
  return true;
})()
"#;

const PAGE_INFO_JS: &str = r#"
JSON.stringify((() => {
  const documentElement = document.documentElement;
  const body = document.body || documentElement;
  const viewportWidth = Math.round(window.innerWidth || documentElement.clientWidth || 0);
  const viewportHeight = Math.round(window.innerHeight || documentElement.clientHeight || 0);
  const pageWidth = Math.round(Math.max(
    body.scrollWidth,
    body.offsetWidth,
    documentElement.clientWidth,
    documentElement.scrollWidth,
    documentElement.offsetWidth
  ));
  const pageHeight = Math.round(Math.max(
    body.scrollHeight,
    body.offsetHeight,
    documentElement.clientHeight,
    documentElement.scrollHeight,
    documentElement.offsetHeight
  ));
  const scrollX = Math.round(window.scrollX || window.pageXOffset || 0);
  const scrollY = Math.round(window.scrollY || window.pageYOffset || 0);
  return {
    viewport_width: viewportWidth,
    viewport_height: viewportHeight,
    page_width: pageWidth,
    page_height: pageHeight,
    scroll_x: scrollX,
    scroll_y: scrollY,
    pixels_above: Math.max(0, scrollY),
    pixels_below: Math.max(0, pageHeight - viewportHeight - scrollY),
    pixels_left: Math.max(0, scrollX),
    pixels_right: Math.max(0, pageWidth - viewportWidth - scrollX)
  };
})())
"#;

fn element_eval_js(index: u32, body: &str) -> String {
    format!(
        r#"
(() => {{
  const selector = [
    'a[href]',
    'button',
    'input',
    'textarea',
    'select',
    'details',
    'summary',
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
    '[tabindex]:not([tabindex="-1"])',
    '[contenteditable="true"]',
    '[contenteditable=""]',
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

fn element_action_js(index: u32, action: &str) -> String {
    element_eval_js(index, &format!("{action}\n  return true;"))
}

fn element_function_js(body: &str) -> String {
    format!(
        r#"function() {{
  const el = this;
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  {body}
}}"#
    )
}

fn element_action_function_js(action: &str) -> String {
    element_function_js(&format!("{action}\n  return true;"))
}

const CLICK_ELEMENT_ACTION_JS: &str = r#"const tag = el.tagName ? el.tagName.toLowerCase() : '';
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

fn click_element_js(index: u32) -> String {
    element_action_js(index, CLICK_ELEMENT_ACTION_JS)
}

fn dropdown_options_js(index: u32) -> String {
    element_eval_js(index, DROPDOWN_OPTIONS_BODY_JS)
}

const DROPDOWN_OPTIONS_BODY_JS: &str = r#"
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

fn select_dropdown_option_body_js(text: &str) -> Result<String, BrowserError> {
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

fn select_dropdown_option_js(index: u32, text: &str) -> Result<String, BrowserError> {
    Ok(element_eval_js(
        index,
        &select_dropdown_option_body_js(text)?,
    ))
}

fn scroll_to_text_js(text: &str) -> Result<String, BrowserError> {
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

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser is not connected")]
    NotConnected,
    #[error("Chrome/Chromium executable not found; checked: {0:?}")]
    ExecutableNotFound(Vec<PathBuf>),
    #[error("browser launch failed: {0}")]
    LaunchFailed(String),
    #[error("timed out waiting for DevToolsActivePort at {0}")]
    DevToolsEndpointTimedOut(PathBuf),
    #[error("CDP transport error: {0}")]
    Transport(String),
    #[error("CDP command {method} failed: {message}")]
    CommandFailed { method: String, message: String },
    #[error("CDP response for {0} was missing expected data")]
    MissingResponseData(String),
    #[error("navigation failed: {0}")]
    NavigationFailed(String),
    #[error("navigation blocked by browser profile policy: {url} ({reason})")]
    NavigationBlocked { url: String, reason: String },
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub base64_png: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf {
    pub base64_pdf: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FoundElement {
    pub tag_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxySettings {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_debugging_port: Option<u16>,
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    #[serde(default)]
    pub block_ip_addresses: bool,
    #[serde(default)]
    pub viewport: BrowserViewport,
    #[serde(default = "default_browser_start_timeout_ms")]
    pub browser_start_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
}

impl Default for BrowserProfile {
    fn default() -> Self {
        Self {
            cdp_url: None,
            executable_path: None,
            remote_debugging_port: None,
            headless: default_headless(),
            user_data_dir: None,
            args: Vec::new(),
            allowed_domains: Vec::new(),
            prohibited_domains: Vec::new(),
            block_ip_addresses: false,
            viewport: BrowserViewport::default(),
            browser_start_timeout_ms: default_browser_start_timeout_ms(),
            proxy: None,
        }
    }
}

fn default_headless() -> bool {
    true
}

fn default_browser_start_timeout_ms() -> u64 {
    10_000
}

impl BrowserProfile {
    pub fn resolve_executable(&self) -> Result<PathBuf, BrowserError> {
        resolve_chrome_executable(
            self.executable_path.as_deref(),
            std::env::var_os("BROWSER_USE_CHROME").map(PathBuf::from),
            default_chrome_candidates(),
        )
    }

    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        let remote_debugging_port = self.remote_debugging_port.unwrap_or(0);
        let mut args = vec![
            format!("--remote-debugging-port={remote_debugging_port}"),
            "--no-first-run".to_owned(),
            "--no-default-browser-check".to_owned(),
            format!(
                "--window-size={},{}",
                self.viewport.width, self.viewport.height
            ),
        ];

        if self.headless {
            args.push("--headless=new".to_owned());
        }

        if let Some(user_data_dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", user_data_dir.display()));
        }

        if let Some(proxy) = &self.proxy {
            args.push(format!("--proxy-server={}", proxy.server));
        }

        args.extend(self.args.iter().cloned());

        BrowserLaunchPlan {
            executable_path: self.executable_path.clone(),
            args,
        }
    }

    pub async fn launch_local(&self) -> Result<LaunchedBrowser, BrowserError> {
        let executable_path = self.resolve_executable()?;
        let (user_data_dir, owned_user_data_dir) = match &self.user_data_dir {
            Some(path) => (path.clone(), None),
            None => {
                let temp_dir = tempfile::Builder::new()
                    .prefix("browser-use-rs-")
                    .tempdir()
                    .map_err(|error| BrowserError::LaunchFailed(error.to_string()))?;
                (temp_dir.path().to_path_buf(), Some(temp_dir))
            }
        };

        let mut launch_profile = self.clone();
        launch_profile.executable_path = Some(executable_path.clone());
        launch_profile.user_data_dir = Some(user_data_dir.clone());
        let plan = launch_profile.launch_plan();

        let mut command = Command::new(&executable_path);
        command
            .args(&plan.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|error| BrowserError::LaunchFailed(error.to_string()))?;

        match wait_for_devtools_endpoint(&user_data_dir, self.browser_start_timeout_ms).await {
            Ok(endpoint) => Ok(LaunchedBrowser {
                child,
                endpoint,
                _user_data_dir: owned_user_data_dir,
            }),
            Err(error) => {
                let _ = child.start_kill();
                Err(error)
            }
        }
    }
}

pub fn resolve_chrome_executable<I>(
    explicit_path: Option<&Path>,
    env_override: Option<PathBuf>,
    candidates: I,
) -> Result<PathBuf, BrowserError>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut checked = Vec::new();

    if let Some(path) = explicit_path {
        checked.push(path.to_path_buf());
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }

    if let Some(path) = env_override {
        checked.push(path.clone());
        if path.exists() {
            return Ok(path);
        }
    }

    for path in candidates {
        checked.push(path.clone());
        if path.exists() {
            return Ok(path);
        }
    }

    Err(BrowserError::ExecutableNotFound(checked))
}

#[must_use]
pub fn default_chrome_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/google-chrome"));
        candidates.push(PathBuf::from("/usr/bin/google-chrome-stable"));
        candidates.push(PathBuf::from("/usr/bin/chromium"));
        candidates.push(PathBuf::from("/usr/bin/chromium-browser"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = std::env::var_os("PROGRAMFILES") {
            candidates
                .push(PathBuf::from(program_files).join("Google/Chrome/Application/chrome.exe"));
        }
        if let Some(program_files_x86) = std::env::var_os("PROGRAMFILES(X86)") {
            candidates.push(
                PathBuf::from(program_files_x86).join("Google/Chrome/Application/chrome.exe"),
            );
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates
                .push(PathBuf::from(local_app_data).join("Google/Chrome/Application/chrome.exe"));
        }
    }

    candidates
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLaunchPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DevToolsEndpoint {
    pub http_url: String,
    pub websocket_url: String,
}

impl DevToolsEndpoint {
    pub fn from_active_port_file(
        host: &str,
        active_port_contents: &str,
    ) -> Result<Self, BrowserError> {
        let mut lines = active_port_contents.lines();
        let port = lines
            .next()
            .ok_or_else(|| {
                BrowserError::StateUnavailable("DevToolsActivePort missing port".to_owned())
            })?
            .trim();
        let browser_path = lines
            .next()
            .ok_or_else(|| {
                BrowserError::StateUnavailable("DevToolsActivePort missing browser path".to_owned())
            })?
            .trim();

        if port.is_empty() || browser_path.is_empty() {
            return Err(BrowserError::StateUnavailable(
                "DevToolsActivePort contains empty endpoint fields".to_owned(),
            ));
        }

        Ok(Self {
            http_url: format!("http://{host}:{port}"),
            websocket_url: format!("ws://{host}:{port}{browser_path}"),
        })
    }
}

pub struct LaunchedBrowser {
    child: Child,
    endpoint: DevToolsEndpoint,
    _user_data_dir: Option<TempDir>,
}

impl LaunchedBrowser {
    #[must_use]
    pub fn endpoint(&self) -> &DevToolsEndpoint {
        &self.endpoint
    }

    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    #[must_use]
    pub fn detach(self) -> DevToolsEndpoint {
        let this = ManuallyDrop::new(self);
        this.endpoint.clone()
    }
}

impl Drop for LaunchedBrowser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[must_use]
pub fn devtools_active_port_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("DevToolsActivePort")
}

pub async fn wait_for_devtools_endpoint(
    user_data_dir: &Path,
    timeout_ms: u64,
) -> Result<DevToolsEndpoint, BrowserError> {
    let active_port_path = devtools_active_port_path(user_data_dir);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        match tokio::fs::read_to_string(&active_port_path).await {
            Ok(contents) => match DevToolsEndpoint::from_active_port_file("127.0.0.1", &contents) {
                Ok(endpoint) => return Ok(endpoint),
                Err(error @ BrowserError::StateUnavailable(_)) => {
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if Instant::now() >= deadline {
                    return Err(BrowserError::DevToolsEndpointTimedOut(active_port_path));
                }
                sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(BrowserError::StateUnavailable(error.to_string())),
        }
    }
}

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct CdpConnection {
    socket: Mutex<CdpSocket>,
    next_id: AtomicU64,
}

impl CdpConnection {
    pub async fn connect(endpoint: &DevToolsEndpoint) -> Result<Self, BrowserError> {
        let (socket, _) = connect_async(&endpoint.websocket_url)
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        Ok(Self {
            socket: Mutex::new(socket),
            next_id: AtomicU64::new(1),
        })
    }

    pub async fn command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, BrowserError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        if let Some(session_id) = session_id {
            request["sessionId"] = Value::String(session_id.to_owned());
        }

        let mut socket = self.socket.lock().await;
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| BrowserError::Transport(error.to_string()))?;
            let Message::Text(text) = message else {
                continue;
            };
            let payload: Value = serde_json::from_str(&text)
                .map_err(|error| BrowserError::Transport(error.to_string()))?;

            if payload.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }

            if let Some(error) = payload.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown CDP error")
                    .to_owned();
                return Err(BrowserError::CommandFailed {
                    method: method.to_owned(),
                    message,
                });
            }

            return payload
                .get("result")
                .cloned()
                .ok_or_else(|| BrowserError::MissingResponseData(format!("{method} result")));
        }

        Err(BrowserError::Transport(
            "CDP websocket closed while waiting for response".to_owned(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPage {
    pub target_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UrlAccessPolicy {
    allowed_domains: Vec<String>,
    prohibited_domains: Vec<String>,
    block_ip_addresses: bool,
}

impl UrlAccessPolicy {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            allowed_domains: profile.allowed_domains.clone(),
            prohibited_domains: profile.prohibited_domains.clone(),
            block_ip_addresses: profile.block_ip_addresses,
        }
    }

    fn validate(&self, url: &str) -> Result<(), BrowserError> {
        if self.is_allowed(url) {
            return Ok(());
        }

        Err(BrowserError::NavigationBlocked {
            url: url.to_owned(),
            reason: self.block_reason(url).to_owned(),
        })
    }

    fn is_unrestricted(&self) -> bool {
        self.allowed_domains.is_empty()
            && self.prohibited_domains.is_empty()
            && !self.block_ip_addresses
    }

    fn is_allowed(&self, url: &str) -> bool {
        if is_internal_browser_url(url) {
            return true;
        }

        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };

        if matches!(parsed.scheme(), "data" | "blob") {
            return true;
        }

        let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };

        if self.block_ip_addresses && is_ip_address(&host) {
            return false;
        }

        if self.allowed_domains.is_empty() && self.prohibited_domains.is_empty() {
            return true;
        }

        if !self.allowed_domains.is_empty() {
            return self
                .allowed_domains
                .iter()
                .any(|pattern| is_url_pattern_match(url, &host, parsed.scheme(), pattern));
        }

        !self
            .prohibited_domains
            .iter()
            .any(|pattern| is_url_pattern_match(url, &host, parsed.scheme(), pattern))
    }

    fn block_reason(&self, url: &str) -> &'static str {
        let Ok(parsed) = url::Url::parse(url) else {
            return "invalid_url";
        };
        if self.block_ip_addresses
            && parsed
                .host_str()
                .map(str::to_ascii_lowercase)
                .is_some_and(|host| is_ip_address(&host))
        {
            return "ip_address_blocked";
        }
        if !self.allowed_domains.is_empty() {
            return "not_in_allowed_domains";
        }
        "in_prohibited_domains"
    }
}

fn is_internal_browser_url(url: &str) -> bool {
    matches!(
        url,
        "about:blank"
            | "chrome://new-tab-page/"
            | "chrome://new-tab-page"
            | "chrome://newtab/"
            | "chrome://newtab"
    )
}

fn is_ip_address(host: &str) -> bool {
    host.trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok()
}

fn is_url_pattern_match(url: &str, host: &str, scheme: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }

    let url = url.to_ascii_lowercase();
    let full_url_pattern = format!("{scheme}://{host}");

    if pattern.contains('*') {
        if let Some(domain) = pattern.strip_prefix("*.") {
            return matches!(scheme, "http" | "https")
                && (host == domain || host.ends_with(&format!(".{domain}")));
        }

        if pattern.ends_with("/*") && glob_match(&url, &pattern) {
            return true;
        }

        let value = if pattern.contains("://") {
            full_url_pattern.as_str()
        } else {
            host
        };
        return glob_match(value, &pattern);
    }

    if pattern.contains("://") {
        return url.starts_with(&pattern);
    }

    host == pattern || (is_root_domain(&pattern) && host == format!("www.{pattern}"))
}

fn is_root_domain(domain: &str) -> bool {
    !domain.contains('*') && !domain.contains("://") && domain.matches('.').count() == 1
}

fn glob_match(value: &str, pattern: &str) -> bool {
    let mut remaining = value;
    let mut parts = pattern.split('*').peekable();
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');

    if let Some(first) = parts.next() {
        if anchored_start {
            let Some(rest) = remaining.strip_prefix(first) else {
                return false;
            };
            remaining = rest;
        } else if !first.is_empty() {
            let Some(index) = remaining.find(first) else {
                return false;
            };
            remaining = &remaining[index + first.len()..];
        }
    }

    while let Some(part) = parts.next() {
        if part.is_empty() {
            continue;
        }
        let Some(index) = remaining.find(part) else {
            return false;
        };
        remaining = &remaining[index + part.len()..];
        if parts.peek().is_none() && anchored_end {
            return remaining.is_empty();
        }
    }

    !anchored_end || remaining.is_empty()
}

pub struct CdpBrowserSession {
    connection: CdpConnection,
    page: Mutex<AttachedPage>,
    last_dom_state: Mutex<Option<SerializedDomState>>,
    url_policy: UrlAccessPolicy,
    _launched_browser: Option<LaunchedBrowser>,
}

impl CdpBrowserSession {
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        let connection = CdpConnection::connect(&endpoint).await?;
        let page = attach_or_create_page(&connection).await?;

        Ok(Self {
            connection,
            page: Mutex::new(page),
            last_dom_state: Mutex::new(None),
            url_policy: UrlAccessPolicy::default(),
            _launched_browser: None,
        })
    }

    pub async fn launch(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        let url_policy = UrlAccessPolicy::from_profile(profile);
        let launched_browser = profile.launch_local().await?;
        let connection = CdpConnection::connect(launched_browser.endpoint()).await?;
        let page = attach_or_create_page(&connection).await?;

        Ok(Self {
            connection,
            page: Mutex::new(page),
            last_dom_state: Mutex::new(None),
            url_policy,
            _launched_browser: Some(launched_browser),
        })
    }

    pub async fn close_browser(&self) -> Result<(), BrowserError> {
        self.connection
            .command("Browser.close", json!({}), None)
            .await
            .map(|_| ())
    }

    async fn current_page(&self) -> AttachedPage {
        self.page.lock().await.clone()
    }

    async fn set_current_page(&self, page: AttachedPage) {
        *self.page.lock().await = page;
    }

    async fn set_cached_dom_state(&self, dom_state: SerializedDomState) {
        *self.last_dom_state.lock().await = Some(dom_state);
    }

    async fn clear_cached_dom_state(&self) {
        *self.last_dom_state.lock().await = None;
    }

    async fn cached_element(&self, index: u32) -> Option<DomElementRef> {
        self.last_dom_state
            .lock()
            .await
            .as_ref()?
            .selector_map
            .get(&index)
            .cloned()
    }

    async fn evaluate_json(&self, expression: &str) -> Result<Value, BrowserError> {
        self.evaluate_json_with_options(expression, false).await
    }

    async fn evaluate_json_with_command_line_api(
        &self,
        expression: &str,
    ) -> Result<Value, BrowserError> {
        self.evaluate_json_with_options(expression, true).await
    }

    async fn evaluate_json_with_options(
        &self,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                runtime_evaluate_params(expression, include_command_line_api),
                Some(&page.session_id),
            )
            .await?;

        runtime_evaluate_value(result)
    }

    async fn evaluate_effect(&self, expression: String) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let _ = runtime_evaluate_value(result)?;
        Ok(())
    }

    async fn resolve_element_object_id(
        &self,
        page: &AttachedPage,
        element: &DomElementRef,
    ) -> Result<String, BrowserError> {
        let params = if let Some(node_id) = element.node_id {
            json!({ "nodeId": node_id })
        } else if element.backend_node_id != 0 {
            json!({ "backendNodeId": element.backend_node_id })
        } else {
            return Err(BrowserError::MissingResponseData(
                "cached element node id".to_owned(),
            ));
        };

        self.connection
            .command("DOM.resolveNode", params, Some(&page.session_id))
            .await?
            .get("object")
            .and_then(|object| object.get("objectId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM.resolveNode objectId".to_owned()))
    }

    async fn call_element_function(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<(), BrowserError> {
        let _ = self
            .call_element_function_value(element, function_declaration)
            .await?;
        Ok(())
    }

    async fn call_element_function_value(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<Value, BrowserError> {
        let page = self.current_page().await;
        let object_id = self.resolve_element_object_id(&page, element).await?;
        let result = self
            .connection
            .command(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function_declaration,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        runtime_evaluate_value(result)
    }

    async fn page_location(&self) -> Result<(String, String), BrowserError> {
        let value = self
            .evaluate_json("JSON.stringify({ url: location.href, title: document.title })")
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate string value".to_owned())
        })?;
        let page: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        Ok((
            page.get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank")
                .to_owned(),
            page.get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ))
    }

    async fn page_info(&self) -> Result<PageInfo, BrowserError> {
        let value = self.evaluate_json(PAGE_INFO_JS).await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate page info".to_owned())
        })?;
        let page_info: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        page_info_from_value(&page_info)
            .ok_or_else(|| BrowserError::MissingResponseData("page info fields".to_owned()))
    }

    async fn dom_state(&self) -> Result<SerializedDomState, BrowserError> {
        let page = self.current_page().await;
        let value = self
            .evaluate_json_with_command_line_api(INTERACTIVE_ELEMENTS_JS)
            .await?;
        let accessibility = self
            .accessibility_enrichment(&page)
            .await
            .unwrap_or_default();
        let _ = self.evaluate_effect(CLEANUP_AX_REFS_JS.to_owned()).await;
        dom_state_from_interactive_value(&page.target_id, &value, &accessibility)
    }

    async fn accessibility_enrichment(
        &self,
        page: &AttachedPage,
    ) -> Result<BTreeMap<String, AccessibilityNodeInfo>, BrowserError> {
        let snapshot = self
            .connection
            .command(
                "DOMSnapshot.captureSnapshot",
                json!({ "computedStyles": [] }),
                Some(&page.session_id),
            )
            .await?;
        let backend_by_ref = snapshot_backend_ids_by_ax_ref(&snapshot);
        if backend_by_ref.is_empty() {
            return Ok(BTreeMap::new());
        }
        let backend_node_ids = backend_by_ref.values().copied().collect::<Vec<_>>();
        let node_ids_by_backend = self
            .node_ids_by_backend_ids(page, &backend_node_ids)
            .await
            .unwrap_or_default();

        let ax_by_backend = self
            .connection
            .command(
                "Accessibility.getFullAXTree",
                json!({}),
                Some(&page.session_id),
            )
            .await
            .map(|tree| accessibility_nodes_by_backend_id(&tree))
            .unwrap_or_default();

        Ok(backend_by_ref
            .into_iter()
            .map(|(ax_ref, backend_node_id)| {
                let mut info = ax_by_backend
                    .get(&backend_node_id)
                    .cloned()
                    .unwrap_or_default();
                info.backend_node_id = backend_node_id;
                info.node_id = node_ids_by_backend.get(&backend_node_id).copied();
                (ax_ref, info)
            })
            .collect())
    }

    async fn node_ids_by_backend_ids(
        &self,
        page: &AttachedPage,
        backend_node_ids: &[u64],
    ) -> Result<BTreeMap<u64, u64>, BrowserError> {
        if backend_node_ids.is_empty() {
            return Ok(BTreeMap::new());
        }

        let _ = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await;
        let result = self
            .connection
            .command(
                "DOM.pushNodesByBackendIdsToFrontend",
                json!({ "backendNodeIds": backend_node_ids }),
                Some(&page.session_id),
            )
            .await?;
        let node_ids = result
            .get("nodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM nodeIds".to_owned()))?;

        Ok(backend_node_ids
            .iter()
            .zip(node_ids)
            .filter_map(|(backend_node_id, node_id)| {
                let node_id = node_id.as_u64()?;
                (node_id != 0).then_some((*backend_node_id, node_id))
            })
            .collect())
    }

    async fn enforce_url_policy_after_settle(&self) -> Result<(), BrowserError> {
        if self.url_policy.is_unrestricted() {
            return Ok(());
        }

        sleep(Duration::from_millis(URL_POLICY_SETTLE_MS)).await;
        self.enforce_open_tab_url_policy().await
    }

    async fn enforce_open_tab_url_policy(&self) -> Result<(), BrowserError> {
        if self.url_policy.is_unrestricted() {
            return Ok(());
        }

        let tabs = page_tabs(&self.connection).await?;
        let current_page = self.current_page().await;
        let mut blocked: Option<BrowserError> = None;

        for tab in tabs {
            if self.url_policy.is_allowed(&tab.url) {
                continue;
            }

            let reason = self.url_policy.block_reason(&tab.url).to_owned();
            if tab.target_id == current_page.target_id {
                let _ = self
                    .connection
                    .command(
                        "Page.navigate",
                        json!({ "url": "about:blank" }),
                        Some(&current_page.session_id),
                    )
                    .await;
            } else {
                self.connection
                    .command(
                        "Target.closeTarget",
                        json!({ "targetId": &tab.target_id }),
                        None,
                    )
                    .await?;
            }
            self.clear_cached_dom_state().await;

            if blocked.is_none() {
                blocked = Some(BrowserError::NavigationBlocked {
                    url: tab.url,
                    reason,
                });
            }
        }

        if let Some(error) = blocked {
            return Err(error);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AccessibilityNodeInfo {
    backend_node_id: u64,
    node_id: Option<u64>,
    role: Option<String>,
    name: Option<String>,
    properties: BTreeMap<String, String>,
}

fn dom_state_from_interactive_value(
    target_id: &str,
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<SerializedDomState, BrowserError> {
    let stats = value
        .get("stats")
        .and_then(dom_page_stats_from_value)
        .unwrap_or_default();
    let element_values = value
        .as_array()
        .or_else(|| value.get("elements").and_then(Value::as_array))
        .ok_or_else(|| BrowserError::MissingResponseData("interactive element array".to_owned()))?;
    let elements = element_values
        .iter()
        .map(|element| dom_element_from_value(target_id, element, accessibility))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(SerializedDomState::from_elements(elements).with_page_stats(stats))
}

fn dom_page_stats_from_value(value: &Value) -> Option<DomPageStats> {
    Some(DomPageStats {
        links: u32_field(value, "links").unwrap_or_default(),
        iframes: u32_field(value, "iframes").unwrap_or_default(),
        shadow_open: u32_field(value, "shadow_open").unwrap_or_default(),
        shadow_closed: u32_field(value, "shadow_closed").unwrap_or_default(),
        scroll_containers: u32_field(value, "scroll_containers").unwrap_or_default(),
        images: u32_field(value, "images").unwrap_or_default(),
        interactive_elements: u32_field(value, "interactive_elements").unwrap_or_default(),
        total_elements: u32_field(value, "total_elements").unwrap_or_default(),
        text_chars: u32_field(value, "text_chars").unwrap_or_default(),
    })
}

fn dom_element_from_value(
    target_id: &str,
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<DomElementRef, BrowserError> {
    let index = value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| BrowserError::MissingResponseData("element index".to_owned()))?;
    let mut attributes: BTreeMap<String, String> = value
        .get("attributes")
        .and_then(Value::as_object)
        .map(|attrs| {
            attrs
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let ax_info = value
        .get("ax_ref")
        .and_then(Value::as_str)
        .and_then(|ax_ref| accessibility.get(ax_ref));
    if let Some(ax_info) = ax_info {
        attributes.extend(ax_info.properties.clone());
    }
    let ax_role = ax_info.and_then(|info| info.role.as_deref());
    let dom_role = value.get("role").and_then(Value::as_str).map(str::to_owned);
    let role = dom_role.or_else(|| {
        ax_role
            .filter(|role| is_useful_ax_role(role))
            .map(str::to_owned)
    });
    let name = ax_info
        .and_then(|info| nonempty_value(info.name.as_deref()))
        .map(str::to_owned)
        .or_else(|| value.get("name").and_then(Value::as_str).map(str::to_owned));

    Ok(DomElementRef {
        index,
        target_id: target_id.to_owned(),
        backend_node_id: ax_info.map(|info| info.backend_node_id).unwrap_or_default(),
        node_id: ax_info.and_then(|info| info.node_id),
        tag_name: value
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or("element")
            .to_owned(),
        role,
        name,
        text: value.get("text").and_then(Value::as_str).map(str::to_owned),
        attributes,
        bounds: element_bounds_from_value(value),
        is_visible: value
            .get("is_visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_interactive: value
            .get("is_interactive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_scrollable: value
            .get("is_scrollable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn snapshot_backend_ids_by_ax_ref(snapshot: &Value) -> BTreeMap<String, u64> {
    let strings = snapshot
        .get("strings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut backend_by_ref = BTreeMap::new();

    for document in snapshot
        .get("documents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(nodes) = document.get("nodes") else {
            continue;
        };
        let backend_node_ids = nodes
            .get("backendNodeId")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let attributes = nodes
            .get("attributes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for (node_index, node_attributes) in attributes.iter().enumerate() {
            let Some(backend_node_id) = backend_node_ids.get(node_index).and_then(Value::as_u64)
            else {
                continue;
            };
            if let Some(ax_ref) =
                snapshot_attribute_value(node_attributes, &strings, AX_REF_ATTRIBUTE)
            {
                backend_by_ref.insert(ax_ref.to_owned(), backend_node_id);
            }
        }
    }

    backend_by_ref
}

fn snapshot_attribute_value<'a>(
    attributes: &'a Value,
    strings: &'a [Value],
    attribute_name: &str,
) -> Option<&'a str> {
    let attributes = attributes.as_array()?;
    for pair in attributes.chunks(2) {
        let [name, value] = pair else {
            continue;
        };
        if snapshot_string(strings, name) == Some(attribute_name) {
            return snapshot_string(strings, value);
        }
    }
    None
}

fn snapshot_string<'a>(strings: &'a [Value], index: &Value) -> Option<&'a str> {
    let index = usize::try_from(index.as_u64()?).ok()?;
    strings.get(index)?.as_str()
}

fn accessibility_nodes_by_backend_id(tree: &Value) -> BTreeMap<u64, AccessibilityNodeInfo> {
    tree.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|node| node.get("ignored").and_then(Value::as_bool) != Some(true))
        .filter_map(|node| {
            let backend_node_id = node.get("backendDOMNodeId").and_then(Value::as_u64)?;
            Some((
                backend_node_id,
                AccessibilityNodeInfo {
                    backend_node_id,
                    node_id: None,
                    role: ax_property_value(node, "role").map(str::to_owned),
                    name: ax_property_value(node, "name").map(str::to_owned),
                    properties: ax_node_properties(node),
                },
            ))
        })
        .collect()
}

fn ax_node_properties(node: &Value) -> BTreeMap<String, String> {
    node.get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|property| {
            let name = property.get("name")?.as_str()?.to_owned();
            let value = ax_property_to_string(property)?;
            Some((name, value))
        })
        .collect()
}

fn ax_property_value<'a>(node: &'a Value, property: &str) -> Option<&'a str> {
    nonempty_value(node.get(property)?.get("value")?.as_str())
}

fn ax_property_to_string(property: &Value) -> Option<String> {
    match property.get("value")?.get("value")? {
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => nonempty_value(Some(value)).map(str::to_owned),
        _ => None,
    }
}

fn nonempty_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn is_useful_ax_role(role: &str) -> bool {
    !matches!(role, "generic" | "none" | "presentation" | "StaticText")
}

fn should_fallback_to_index_traversal(error: &BrowserError) -> bool {
    match error {
        BrowserError::MissingResponseData(message) => message.contains("cached element node id"),
        BrowserError::CommandFailed { method, message } => {
            method == "DOM.resolveNode"
                && (message.contains("No node")
                    || message.contains("Could not find")
                    || message.contains("Invalid remote object id"))
        }
        _ => false,
    }
}

fn parse_dropdown_options_value(value: Value) -> Result<Vec<String>, BrowserError> {
    let encoded = value
        .as_str()
        .ok_or_else(|| BrowserError::MissingResponseData("dropdown options string".to_owned()))?;
    serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
}

fn element_bounds_from_value(value: &Value) -> Option<ElementBounds> {
    let bounds = value.get("bounds")?;
    Some(ElementBounds {
        x: bounds
            .get("x")?
            .as_i64()
            .and_then(|x| i32::try_from(x).ok())?,
        y: bounds
            .get("y")?
            .as_i64()
            .and_then(|y| i32::try_from(y).ok())?,
        width: bounds
            .get("width")?
            .as_u64()
            .and_then(|width| u32::try_from(width).ok())?,
        height: bounds
            .get("height")?
            .as_u64()
            .and_then(|height| u32::try_from(height).ok())?,
    })
}

fn page_info_from_value(value: &Value) -> Option<PageInfo> {
    Some(PageInfo {
        viewport_width: u32_field(value, "viewport_width")?,
        viewport_height: u32_field(value, "viewport_height")?,
        page_width: u32_field(value, "page_width")?,
        page_height: u32_field(value, "page_height")?,
        scroll_x: i32_field(value, "scroll_x")?,
        scroll_y: i32_field(value, "scroll_y")?,
        pixels_above: u32_field(value, "pixels_above")?,
        pixels_below: u32_field(value, "pixels_below")?,
        pixels_left: u32_field(value, "pixels_left")?,
        pixels_right: u32_field(value, "pixels_right")?,
    })
}

fn detect_pagination_buttons(dom_state: &SerializedDomState) -> Vec<PaginationButton> {
    let mut buttons = Vec::new();

    for element in dom_state.selector_map.values() {
        if !element.is_interactive {
            continue;
        }

        let label = pagination_label_text(element);
        let label_lower = label.to_lowercase();
        let role = element
            .role
            .as_deref()
            .or_else(|| element.attributes.get("role").map(String::as_str))
            .unwrap_or("")
            .to_ascii_lowercase();

        let button_type = if contains_any(
            &label_lower,
            &["first", "⇤", "primera", "première", "erste", "eerste"],
        ) {
            Some(PaginationButtonType::First)
        } else if contains_any(
            &label_lower,
            &["last", "⇥", "última", "dernier", "letzte", "laatste"],
        ) {
            Some(PaginationButtonType::Last)
        } else if contains_any(
            &label_lower,
            &[
                "next",
                ">",
                "›",
                "→",
                "»",
                "siguiente",
                "suivant",
                "volgende",
            ],
        ) {
            Some(PaginationButtonType::Next)
        } else if contains_any(
            &label_lower,
            &[
                "prev",
                "previous",
                "<",
                "‹",
                "←",
                "«",
                "anterior",
                "précédent",
                "vorige",
            ],
        ) {
            Some(PaginationButtonType::Prev)
        } else if label_lower.trim().len() <= 2
            && label_lower
                .trim()
                .chars()
                .all(|character| character.is_ascii_digit())
            && matches!(role.as_str(), "" | "button" | "link")
        {
            Some(PaginationButtonType::PageNumber)
        } else {
            None
        };

        let Some(button_type) = button_type else {
            continue;
        };

        buttons.push(PaginationButton {
            button_type,
            backend_node_id: if element.backend_node_id == 0 {
                u64::from(element.index)
            } else {
                element.backend_node_id
            },
            text: label.trim().to_owned(),
            selector: pagination_selector(element),
            is_disabled: pagination_is_disabled(element),
        });
    }

    buttons
}

fn pagination_label_text(element: &DomElementRef) -> String {
    let mut parts = vec![render_element_text(element)];
    for attribute in ["aria-label", "title", "class"] {
        if let Some(value) = element.attributes.get(attribute) {
            parts.push(value.clone());
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn pagination_is_disabled(element: &DomElementRef) -> bool {
    element
        .attributes
        .get("disabled")
        .is_some_and(|value| value == "true" || value.is_empty())
        || element
            .attributes
            .get("aria-disabled")
            .is_some_and(|value| value == "true")
        || element
            .attributes
            .get("class")
            .is_some_and(|value| value.to_lowercase().contains("disabled"))
}

fn pagination_selector(element: &DomElementRef) -> String {
    if let Some(id) = element.attributes.get("id") {
        format!("#{id}")
    } else if let Some(name) = element.attributes.get("name") {
        format!("{}[name=\"{}\"]", element.tag_name, name)
    } else {
        format!("{}:nth-index({})", element.tag_name, element.index)
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn u32_field(value: &Value, field: &str) -> Option<u32> {
    value
        .get(field)?
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
}

fn i32_field(value: &Value, field: &str) -> Option<i32> {
    value
        .get(field)?
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
}

fn normalize_send_keys(keys: &str) -> String {
    if keys.contains('+') {
        return keys
            .split('+')
            .map(normalize_key_alias)
            .collect::<Vec<_>>()
            .join("+");
    }

    normalize_key_or_text(keys)
}

fn normalize_key_alias(key: &str) -> String {
    key_alias(key).unwrap_or_else(|| key.trim().to_owned())
}

fn normalize_key_or_text(key: &str) -> String {
    key_alias(key).unwrap_or_else(|| key.to_owned())
}

fn key_alias(key: &str) -> Option<String> {
    Some(match key.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "Control".to_owned(),
        "alt" | "option" => "Alt".to_owned(),
        "meta" | "cmd" | "command" => "Meta".to_owned(),
        "shift" => "Shift".to_owned(),
        "enter" | "return" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "delete" => "Delete".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "escape" | "esc" => "Escape".to_owned(),
        "space" => " ".to_owned(),
        "up" => "ArrowUp".to_owned(),
        "down" => "ArrowDown".to_owned(),
        "left" => "ArrowLeft".to_owned(),
        "right" => "ArrowRight".to_owned(),
        "pageup" => "PageUp".to_owned(),
        "pagedown" => "PageDown".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        _ => return None,
    })
}

fn is_special_key(key: &str) -> bool {
    matches!(
        key,
        "Enter"
            | "Tab"
            | "Delete"
            | "Backspace"
            | "Escape"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "PageUp"
            | "PageDown"
            | "Home"
            | "End"
            | "Control"
            | "Alt"
            | "Meta"
            | "Shift"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
    )
}

fn modifier_mask(modifiers: &[String]) -> i64 {
    modifiers.iter().fold(0, |mask, modifier| {
        mask | match modifier.as_str() {
            "Alt" => 1,
            "Control" => 2,
            "Meta" => 4,
            "Shift" => 8,
            _ => 0,
        }
    })
}

fn key_info(key: &str) -> (String, Option<i64>) {
    match key {
        "Enter" => ("Enter".to_owned(), Some(13)),
        "Tab" => ("Tab".to_owned(), Some(9)),
        "Delete" => ("Delete".to_owned(), Some(46)),
        "Backspace" => ("Backspace".to_owned(), Some(8)),
        "Escape" => ("Escape".to_owned(), Some(27)),
        "ArrowUp" => ("ArrowUp".to_owned(), Some(38)),
        "ArrowDown" => ("ArrowDown".to_owned(), Some(40)),
        "ArrowLeft" => ("ArrowLeft".to_owned(), Some(37)),
        "ArrowRight" => ("ArrowRight".to_owned(), Some(39)),
        "PageUp" => ("PageUp".to_owned(), Some(33)),
        "PageDown" => ("PageDown".to_owned(), Some(34)),
        "Home" => ("Home".to_owned(), Some(36)),
        "End" => ("End".to_owned(), Some(35)),
        "Control" => ("ControlLeft".to_owned(), Some(17)),
        "Alt" => ("AltLeft".to_owned(), Some(18)),
        "Meta" => ("MetaLeft".to_owned(), Some(91)),
        "Shift" => ("ShiftLeft".to_owned(), Some(16)),
        " " => ("Space".to_owned(), Some(32)),
        function_key if function_key.starts_with('F') => {
            let number = function_key[1..].parse::<i64>().ok();
            if let Some(number @ 1..=12) = number {
                (function_key.to_owned(), Some(111 + number))
            } else {
                (function_key.to_owned(), None)
            }
        }
        single if single.chars().count() == 1 => {
            let lower = single.to_ascii_lowercase();
            let upper = lower.to_ascii_uppercase();
            let vk = upper.as_bytes().first().copied().map(i64::from);
            (format!("Key{upper}"), vk)
        }
        other => (other.to_owned(), None),
    }
}

fn key_event_params(event_type: &str, key: &str, modifiers: i64) -> Value {
    let key = if key.chars().count() == 1 {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    };
    let (code, vk_code) = key_info(&key);
    let mut params = serde_json::Map::new();
    params.insert("type".to_owned(), json!(event_type));
    params.insert("key".to_owned(), json!(key));
    params.insert("code".to_owned(), json!(code));
    if modifiers != 0 {
        params.insert("modifiers".to_owned(), json!(modifiers));
    }
    if let Some(vk_code) = vk_code {
        params.insert("windowsVirtualKeyCode".to_owned(), json!(vk_code));
    }
    Value::Object(params)
}

fn runtime_evaluate_params(expression: &str, include_command_line_api: bool) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("expression".to_owned(), json!(expression));
    params.insert("returnByValue".to_owned(), json!(true));
    params.insert("awaitPromise".to_owned(), json!(true));
    if include_command_line_api {
        params.insert("includeCommandLineAPI".to_owned(), json!(true));
    }
    Value::Object(params)
}

fn runtime_evaluate_value(result: Value) -> Result<Value, BrowserError> {
    if let Some(exception) = result.get("exceptionDetails") {
        let message = exception
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("Runtime.evaluate exception")
            .to_owned();
        return Err(BrowserError::CommandFailed {
            method: "Runtime.evaluate".to_owned(),
            message,
        });
    }

    result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| BrowserError::MissingResponseData("Runtime.evaluate value".to_owned()))
}

fn render_runtime_evaluate_result(result: &Value) -> Result<String, BrowserError> {
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(BrowserError::CommandFailed {
            method: "Runtime.evaluate".to_owned(),
            message: exception
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("Runtime.evaluate exception")
                .to_owned(),
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

async fn attach_or_create_page(connection: &CdpConnection) -> Result<AttachedPage, BrowserError> {
    let targets = connection
        .command("Target.getTargets", json!({}), None)
        .await?;
    let target_infos = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut page_targets: Vec<String> = target_infos
        .iter()
        .filter(|target| {
            target.get("type").and_then(Value::as_str) == Some("page")
                && target.get("url").and_then(Value::as_str) != Some("chrome://newtab/")
        })
        .filter_map(|target| {
            target
                .get("targetId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    page_targets.extend(
        target_infos
            .iter()
            .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
            .filter(|target| target.get("url").and_then(Value::as_str) == Some("chrome://newtab/"))
            .filter_map(|target| {
                target
                    .get("targetId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
    );

    for target_id in page_targets {
        match attach_to_target(connection, target_id).await {
            Ok(page) => return Ok(page),
            Err(BrowserError::CommandFailed { method, message })
                if method == "Target.attachToTarget"
                    && message.contains("No target with given id found") =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    let target_id = create_target(connection, "about:blank").await?;
    attach_to_target(connection, target_id).await
}

async fn create_target(connection: &CdpConnection, url: &str) -> Result<String, BrowserError> {
    connection
        .command("Target.createTarget", json!({ "url": url }), None)
        .await?
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BrowserError::MissingResponseData("Target.createTarget targetId".to_owned()))
}

async fn attach_to_target(
    connection: &CdpConnection,
    target_id: String,
) -> Result<AttachedPage, BrowserError> {
    let session_id = connection
        .command(
            "Target.attachToTarget",
            json!({
                "targetId": target_id,
                "flatten": true,
            }),
            None,
        )
        .await?
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Target.attachToTarget sessionId".to_owned())
        })?;

    Ok(AttachedPage {
        target_id,
        session_id,
    })
}

async fn page_tabs(connection: &CdpConnection) -> Result<Vec<TabInfo>, BrowserError> {
    let targets = connection
        .command("Target.getTargets", json!({}), None)
        .await?;
    let tabs = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            let target_id = target.get("targetId")?.as_str()?.to_owned();
            let tab_id = TabInfo::tab_id_for_target(&target_id);
            Some(TabInfo {
                url: target
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("about:blank")
                    .to_owned(),
                title: target
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                tab_id,
                target_id,
                parent_target_id: None,
            })
        })
        .collect();
    Ok(tabs)
}

fn resolve_page_target_id_from_tabs(
    tabs: &[TabInfo],
    tab_id_or_target_id: &str,
) -> Result<String, BrowserError> {
    if let Some(tab) = tabs.iter().find(|tab| tab.target_id == tab_id_or_target_id) {
        return Ok(tab.target_id.clone());
    }

    if tab_id_or_target_id.len() == 4 {
        let matches = tabs
            .iter()
            .filter(|tab| tab.short_target_id() == tab_id_or_target_id)
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [tab] => Ok(tab.target_id.clone()),
            [] => Err(BrowserError::ActionFailed(format!(
                "No open tab found for short tab id {tab_id_or_target_id}"
            ))),
            _ => Err(BrowserError::ActionFailed(format!(
                "Short tab id {tab_id_or_target_id} matched multiple open tabs"
            ))),
        };
    }

    Err(BrowserError::ActionFailed(format!(
        "No open tab found for target id {tab_id_or_target_id}"
    )))
}

async fn resolve_page_target_id(
    connection: &CdpConnection,
    tab_id_or_target_id: &str,
) -> Result<String, BrowserError> {
    let tabs = page_tabs(connection).await?;
    resolve_page_target_id_from_tabs(&tabs, tab_id_or_target_id)
}

#[async_trait]
impl BrowserSession for CdpBrowserSession {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.enforce_open_tab_url_policy().await?;
        let (url, title) = self.page_location().await?;
        let page_info = self.page_info().await?;
        let dom_state = self.dom_state().await?;
        self.set_cached_dom_state(dom_state.clone()).await;
        let pagination_buttons = detect_pagination_buttons(&dom_state);
        let current_page = self.current_page().await;
        let tabs = page_tabs(&self.connection).await?;
        let screenshot = if include_screenshot {
            Some(self.screenshot().await?.base64_png)
        } else {
            None
        };

        Ok(BrowserStateSummary {
            dom_state,
            url: url.clone(),
            title: title.clone(),
            tabs: if tabs.is_empty() {
                vec![TabInfo {
                    url,
                    title,
                    tab_id: TabInfo::tab_id_for_target(&current_page.target_id),
                    target_id: current_page.target_id,
                    parent_target_id: None,
                }]
            } else {
                tabs
            },
            screenshot,
            page_info: Some(page_info),
            pixels_above: page_info.pixels_above,
            pixels_below: page_info.pixels_below,
            browser_errors: vec![],
            is_pdf_viewer: false,
            recent_events: None,
            pending_network_requests: vec![],
            pagination_buttons,
            closed_popup_messages: vec![],
        })
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.url_policy.validate(url)?;
        if new_tab {
            let target_id = create_target(&self.connection, url).await?;
            let page = attach_to_target(&self.connection, target_id).await?;
            self.set_current_page(page).await;
            self.clear_cached_dom_state().await;
            return self.enforce_url_policy_after_settle().await;
        }

        let page = self.current_page().await;
        self.connection
            .command(
                "Page.navigate",
                json!({
                    "url": url,
                }),
                Some(&page.session_id),
            )
            .await?;
        self.clear_cached_dom_state().await;
        self.enforce_url_policy_after_settle().await
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let history = self
            .connection
            .command(
                "Page.getNavigationHistory",
                json!({}),
                Some(&page.session_id),
            )
            .await?;
        let entry_id = previous_navigation_entry_id(&history)?;
        self.connection
            .command(
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry_id }),
                Some(&page.session_id),
            )
            .await?;
        self.clear_cached_dom_state().await;
        self.enforce_url_policy_after_settle().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        let page = attach_to_target(&self.connection, target_id).await?;
        self.set_current_page(page).await;
        self.clear_cached_dom_state().await;
        self.enforce_open_tab_url_policy().await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        self.connection
            .command(
                "Target.closeTarget",
                json!({ "targetId": &target_id }),
                None,
            )
            .await?;

        if self.current_page().await.target_id == target_id {
            let page = attach_or_create_page(&self.connection).await?;
            self.set_current_page(page).await;
        }
        self.clear_cached_dom_state().await;

        self.enforce_open_tab_url_policy().await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        if let Some(element) = self.cached_element(index).await {
            match self
                .call_element_function(
                    &element,
                    element_action_function_js(CLICK_ELEMENT_ACTION_JS),
                )
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect(click_element_js(index)).await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        for event_type in ["mousePressed", "mouseReleased"] {
            self.connection
                .command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                    Some(&page.session_id),
                )
                .await?;
        }
        self.enforce_url_policy_after_settle().await
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        let text_json = serde_json::to_string(text)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let action = if clear {
            format!(
                "el.focus(); el.value = {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        } else {
            format!(
                "el.focus(); el.value = (el.value || '') + {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        };
        if let Some(element) = self.cached_element(index).await {
            match self
                .call_element_function(&element, element_action_function_js(&action))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect(element_action_js(index, &action))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        let direction = if down { 1.0 } else { -1.0 };
        if let Some(index) = index {
            let action = format!(
                "el.scrollBy(0, (el.clientHeight || window.innerHeight) * {});",
                pages * direction
            );
            if let Some(element) = self.cached_element(index).await {
                match self
                    .call_element_function(&element, element_action_function_js(&action))
                    .await
                {
                    Ok(()) => return self.enforce_url_policy_after_settle().await,
                    Err(error) if should_fallback_to_index_traversal(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            self.evaluate_effect(element_action_js(index, &action))
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }
        self.evaluate_effect(format!(
            "window.scrollBy(0, window.innerHeight * {}); true;",
            pages * direction
        ))
        .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        let found = self
            .evaluate_json(&scroll_to_text_js(text)?)
            .await?
            .as_bool()
            .ok_or_else(|| BrowserError::MissingResponseData("scroll-to-text result".to_owned()))?;
        self.enforce_url_policy_after_settle().await?;
        Ok(found)
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": code,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let rendered = render_runtime_evaluate_result(&result)?;
        self.enforce_url_policy_after_settle().await?;
        Ok(rendered)
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        if let Some(element) = self.cached_element(index).await {
            match self
                .call_element_function_value(
                    &element,
                    element_function_js(DROPDOWN_OPTIONS_BODY_JS),
                )
                .await
            {
                Ok(value) => return parse_dropdown_options_value(value),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let value = self.evaluate_json(&dropdown_options_js(index)).await?;
        parse_dropdown_options_value(value)
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        let body = select_dropdown_option_body_js(text)?;
        if let Some(element) = self.cached_element(index).await {
            match self
                .call_element_function(&element, element_function_js(&body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect(select_dropdown_option_js(index, text)?)
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        let value = self
            .evaluate_json(
                "(document.body ? document.body.innerText : document.documentElement.innerText || '')",
            )
            .await?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("page text".to_owned()))
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let selector_json = serde_json::to_string(selector)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let attributes_json = serde_json::to_string(attributes)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let value = self
            .evaluate_json(&format!(
                r#"
JSON.stringify((() => {{
  const selector = {selector_json};
  const attributeNames = {attributes_json};
  return Array.from(document.querySelectorAll(selector)).slice(0, {max_results}).map((el) => {{
    const attrs = {{}};
    for (const name of attributeNames) {{
      const value = el.getAttribute(name);
      if (value !== null && value !== '') attrs[name] = value;
    }}
    return {{
      tag_name: el.tagName.toLowerCase(),
      text: {text_expr},
      attributes: attrs
    }};
  }});
}})())
"#,
                selector_json = selector_json,
                attributes_json = attributes_json,
                max_results = max_results,
                text_expr = if include_text {
                    "(el.innerText || el.value || '').trim().slice(0, 500)"
                } else {
                    "null"
                }
            ))
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("find elements result string".to_owned())
        })?;
        serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let normalized_keys = normalize_send_keys(keys);
        if normalized_keys.contains('+') {
            let parts = normalized_keys
                .split('+')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some((main_key, modifiers)) = parts.split_last() {
                let modifier_value = modifier_mask(modifiers);
                for modifier in modifiers {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyDown", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyDown", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyUp", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                for modifier in modifiers.iter().rev() {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyUp", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
            }
            return self.enforce_url_policy_after_settle().await;
        }

        if is_special_key(&normalized_keys) {
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyDown", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            if normalized_keys == "Enter" {
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "char",
                            "text": "\r",
                            "key": "Enter",
                        }),
                        Some(&page.session_id),
                    )
                    .await?;
            }
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyUp", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }

        self.connection
            .command(
                "Input.insertText",
                json!({
                    "text": normalized_keys,
                }),
                Some(&page.session_id),
            )
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        let canonical_path = std::fs::canonicalize(path).map_err(|error| {
            BrowserError::ActionFailed(format!(
                "failed to resolve upload file '{}': {error}",
                path.display()
            ))
        })?;
        if !canonical_path.is_file() {
            return Err(BrowserError::ActionFailed(format!(
                "upload path is not a file: {}",
                canonical_path.display()
            )));
        }
        let path_string = canonical_path.to_str().ok_or_else(|| {
            BrowserError::ActionFailed(format!(
                "upload path is not valid UTF-8: {}",
                canonical_path.display()
            ))
        })?;

        let token = format!(
            "browser-use-rs-upload-{}",
            self.connection.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let token_json = serde_json::to_string(&token)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let mark_upload_body = format!(
            r#"
  if (el.tagName.toLowerCase() !== 'input' || el.type !== 'file') {{
    throw new Error('Element is not a file input');
  }}
  el.setAttribute('data-browser-use-rs-upload-token', {token_json});
  return true;
"#
        );
        let cached_element = self.cached_element(index).await;
        let mut used_cached_element = false;
        if let Some(element) = cached_element.as_ref() {
            match self
                .call_element_function(element, element_function_js(&mark_upload_body))
                .await
            {
                Ok(()) => used_cached_element = true,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        if !used_cached_element {
            self.evaluate_effect(element_eval_js(index, &mark_upload_body))
                .await?;
        }

        let page = self.current_page().await;
        let document = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await?;
        let root_node_id = document
            .get("root")
            .and_then(|root| u32_field(root, "nodeId"))
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.getDocument root nodeId".to_owned())
            })?;
        let selector = format!(r#"[data-browser-use-rs-upload-token="{token}"]"#);
        let query_result = self
            .connection
            .command(
                "DOM.querySelector",
                json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                }),
                Some(&page.session_id),
            )
            .await?;
        let node_id = u32_field(&query_result, "nodeId")
            .filter(|node_id| *node_id != 0)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.querySelector nodeId".to_owned())
            })?;

        self.connection
            .command(
                "DOM.setFileInputFiles",
                json!({
                    "nodeId": node_id,
                    "files": [path_string],
                }),
                Some(&page.session_id),
            )
            .await?;

        let finish_upload_body = r#"
  el.removeAttribute('data-browser-use-rs-upload-token');
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
"#;
        if used_cached_element {
            if let Some(element) = cached_element.as_ref() {
                match self
                    .call_element_function(element, element_function_js(finish_upload_body))
                    .await
                {
                    Ok(()) => return self.enforce_url_policy_after_settle().await,
                    Err(error) if should_fallback_to_index_traversal(&error) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        self.evaluate_effect(element_eval_js(index, finish_upload_body))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "fromSurface": true,
                }),
                Some(&page.session_id),
            )
            .await?;

        let base64_png = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("Page.captureScreenshot data".to_owned())
            })?;

        Ok(Screenshot { base64_png })
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        let page = self.current_page().await;
        let (paper_width, paper_height) = paper_size_inches(paper_format);
        let result = self
            .connection
            .command(
                "Page.printToPDF",
                json!({
                    "printBackground": print_background,
                    "landscape": landscape,
                    "scale": scale,
                    "paperWidth": paper_width,
                    "paperHeight": paper_height,
                }),
                Some(&page.session_id),
            )
            .await?;

        let base64_pdf = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("Page.printToPDF data".to_owned()))?;

        Ok(Pdf { base64_pdf })
    }
}

fn paper_size_inches(format: &str) -> (f64, f64) {
    match format.to_ascii_lowercase().as_str() {
        "a4" => (8.27, 11.69),
        "legal" => (8.5, 14.0),
        "tabloid" => (11.0, 17.0),
        _ => (8.5, 11.0),
    }
}

fn previous_navigation_entry_id(history: &Value) -> Result<i64, BrowserError> {
    let current_index = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Page.getNavigationHistory currentIndex".to_owned())
        })?;

    if current_index <= 0 {
        return Err(BrowserError::ActionFailed(
            "No previous browser history entry".to_owned(),
        ));
    }

    history
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.get((current_index - 1) as usize))
        .and_then(|entry| entry.get("id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Page.getNavigationHistory entries".to_owned())
        })
}

#[async_trait]
pub trait BrowserSession: Send + Sync {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    async fn go_back(&self) -> Result<(), BrowserError>;

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError>;

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError>;

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError>;

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError>;

    async fn page_text(&self) -> Result<String, BrowserError>;

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError>;

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError>;

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError>;

    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError>;
}

#[async_trait]
impl<T> BrowserSession for Arc<T>
where
    T: BrowserSession + ?Sized,
{
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.as_ref().state(include_screenshot).await
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.as_ref().navigate(url, new_tab).await
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.as_ref().go_back().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().switch_tab(target_id).await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().close_tab(target_id).await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        self.as_ref().click(index).await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        self.as_ref().click_coordinates(x, y).await
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        self.as_ref().input_text(index, text, clear).await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        self.as_ref().scroll(index, down, pages).await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        self.as_ref().find_text(text).await
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        self.as_ref().evaluate(code).await
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        self.as_ref().dropdown_options(index).await
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        self.as_ref().select_dropdown_option(index, text).await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        self.as_ref().page_text().await
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        self.as_ref()
            .find_elements(selector, attributes, max_results, include_text)
            .await
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        self.as_ref().send_keys(keys).await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        self.as_ref().upload_file(index, path).await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.as_ref().screenshot().await
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        self.as_ref()
            .save_pdf(print_background, landscape, scale, paper_format)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_uses_headless_chrome_args() {
        let plan = BrowserProfile::default().launch_plan();

        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn profile_can_pin_remote_debugging_port() {
        let profile = BrowserProfile {
            remote_debugging_port: Some(9222),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(
            plan.args
                .contains(&"--remote-debugging-port=9222".to_owned())
        );
    }

    #[test]
    fn launch_plan_preserves_profile_and_custom_args_order() {
        let profile = BrowserProfile {
            headless: false,
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            args: vec!["--disable-gpu".to_owned()],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(!plan.args.contains(&"--headless=new".to_owned()));
        assert!(
            plan.args
                .contains(&"--user-data-dir=/tmp/browser-use-rs-profile".to_owned())
        );
        assert!(
            plan.args
                .contains(&"--proxy-server=http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn url_policy_allows_internal_data_and_default_web_urls() {
        let policy = UrlAccessPolicy::default();

        assert!(policy.is_allowed("about:blank"));
        assert!(policy.is_allowed("chrome://newtab/"));
        assert!(policy.is_allowed("chrome://new-tab-page"));
        assert!(policy.is_allowed("data:text/html,<title>ok</title>"));
        assert!(policy.is_allowed("blob:https://example.com/id"));
        assert!(policy.is_allowed("https://example.com/page"));
    }

    #[test]
    fn url_policy_matches_allowed_domain_variants_and_wildcards() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec![
                "*.google.com".to_owned(),
                "https://wiki.org".to_owned(),
                "https://*.test.com".to_owned(),
                "chrome://version".to_owned(),
                "brave://*".to_owned(),
            ],
            ..BrowserProfile::default()
        });

        assert!(policy.is_allowed("https://google.com"));
        assert!(policy.is_allowed("https://www.google.com"));
        assert!(policy.is_allowed("https://mail.google.com"));
        assert!(!policy.is_allowed("https://evilgoogle.com"));
        assert!(!policy.is_allowed("chrome://abc.google.com"));
        assert!(!policy.is_allowed("http://wiki.org"));
        assert!(policy.is_allowed("https://wiki.org/page"));
        assert!(policy.is_allowed("https://www.test.com"));
        assert!(!policy.is_allowed("https://www.testx.com"));
        assert!(policy.is_allowed("chrome://version"));
        assert!(!policy.is_allowed("chrome://settings"));
        assert!(policy.is_allowed("brave://anything/"));
    }

    #[test]
    fn url_policy_prevents_allowed_domain_auth_bypass() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned(), "*.google.com".to_owned()],
            ..BrowserProfile::default()
        });

        assert!(!policy.is_allowed("https://example.com:password@malicious.com"));
        assert!(!policy.is_allowed("https://example.com@malicious.com"));
        assert!(!policy.is_allowed("https://example.com%20@malicious.com"));
        assert!(!policy.is_allowed("https://sub.google.com@evil.org"));
        assert!(policy.is_allowed("https://user:password@example.com"));
    }

    #[test]
    fn url_policy_root_domain_www_rules_match_upstream() {
        let simple = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned(), "test.org".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(simple.is_allowed("https://example.com"));
        assert!(simple.is_allowed("https://www.example.com"));
        assert!(!simple.is_allowed("https://mail.example.com"));
        assert!(!simple.is_allowed("https://notexample.com"));

        let country_tld = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.co.uk".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(country_tld.is_allowed("https://example.co.uk"));
        assert!(!country_tld.is_allowed("https://www.example.co.uk"));

        let full_url = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["https://example.com".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(full_url.is_allowed("https://example.com/path"));
        assert!(!full_url.is_allowed("https://www.example.com"));
    }

    #[test]
    fn url_policy_blocks_prohibited_domains_and_preserves_allowlist_precedence() {
        let prohibited_policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec![
                "example.com".to_owned(),
                "*.ads.example".to_owned(),
                "https://tracker.test".to_owned(),
                "brave://*".to_owned(),
            ],
            ..BrowserProfile::default()
        });

        assert!(!prohibited_policy.is_allowed("https://example.com"));
        assert!(!prohibited_policy.is_allowed("https://www.example.com"));
        assert!(prohibited_policy.is_allowed("https://mail.example.com"));
        assert!(!prohibited_policy.is_allowed("https://cdn.ads.example/pixel"));
        assert!(!prohibited_policy.is_allowed("https://tracker.test/collect?id=1"));
        assert!(prohibited_policy.is_allowed("http://tracker.test/collect?id=1"));
        assert!(!prohibited_policy.is_allowed("brave://anything/"));
        assert!(prohibited_policy.is_allowed("chrome://new-tab-page/"));

        let allowlist_wins = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["*.example.com".to_owned()],
            prohibited_domains: vec!["https://example.com".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(allowlist_wins.is_allowed("https://example.com"));
        assert!(allowlist_wins.is_allowed("https://api.example.com"));
        assert!(!allowlist_wins.is_allowed("https://notexample.com"));
    }

    #[test]
    fn url_policy_blocks_ip_addresses_when_configured() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });

        assert!(!policy.is_allowed("http://127.0.0.1:9222/json"));
        assert!(!policy.is_allowed("http://[::1]/"));
        assert!(policy.is_allowed("https://example.com"));
    }

    #[test]
    fn url_policy_treats_ip_blocking_as_restricted() {
        assert!(UrlAccessPolicy::default().is_unrestricted());
        assert!(
            !UrlAccessPolicy::from_profile(&BrowserProfile {
                block_ip_addresses: true,
                ..BrowserProfile::default()
            })
            .is_unrestricted()
        );
        assert!(
            !UrlAccessPolicy::from_profile(&BrowserProfile {
                prohibited_domains: vec!["blocked.test".to_owned()],
                ..BrowserProfile::default()
            })
            .is_unrestricted()
        );
    }

    #[test]
    fn url_policy_validate_reports_block_reason() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned()],
            ..BrowserProfile::default()
        });

        let error = policy
            .validate("https://blocked.test")
            .expect_err("navigation should be blocked");
        assert_eq!(
            error.to_string(),
            "navigation blocked by browser profile policy: https://blocked.test (not_in_allowed_domains)"
        );

        let ip_policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });
        let error = ip_policy
            .validate("http://127.0.0.1/")
            .expect_err("ip navigation should be blocked");
        assert_eq!(
            error.to_string(),
            "navigation blocked by browser profile policy: http://127.0.0.1/ (ip_address_blocked)"
        );
    }

    #[test]
    fn parses_devtools_active_port_endpoint() {
        let endpoint = DevToolsEndpoint::from_active_port_file(
            "127.0.0.1",
            "38119\n/devtools/browser/abc123\n",
        )
        .expect("parse endpoint");

        assert_eq!(endpoint.http_url, "http://127.0.0.1:38119");
        assert_eq!(
            endpoint.websocket_url,
            "ws://127.0.0.1:38119/devtools/browser/abc123"
        );
    }

    #[test]
    fn active_port_path_lives_under_user_data_dir() {
        assert_eq!(
            devtools_active_port_path(Path::new("/tmp/profile")),
            PathBuf::from("/tmp/profile/DevToolsActivePort")
        );
    }

    #[test]
    fn parses_page_info_metrics() {
        let page_info = page_info_from_value(&json!({
            "viewport_width": 1280,
            "viewport_height": 720,
            "page_width": 1280,
            "page_height": 2000,
            "scroll_x": 0,
            "scroll_y": 300,
            "pixels_above": 300,
            "pixels_below": 980,
            "pixels_left": 0,
            "pixels_right": 0
        }))
        .expect("page info");

        assert_eq!(page_info.scroll_y, 300);
        assert_eq!(page_info.pixels_below, 980);
    }

    #[test]
    fn detects_pagination_buttons_from_dom_state() {
        let dom_state = SerializedDomState::from_elements(vec![
            test_dom_element(1, "button", Some("Next"), &[("id", "next")]),
            test_dom_element(2, "a", Some("2"), &[("href", "/page/2"), ("role", "link")]),
            test_dom_element(3, "button", Some("Export"), &[("id", "export")]),
            test_dom_element(4, "button", Some("Previous"), &[("class", "disabled")]),
        ]);

        let buttons = detect_pagination_buttons(&dom_state);

        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0].button_type, PaginationButtonType::Next);
        assert_eq!(buttons[0].selector, "#next");
        assert_eq!(buttons[1].button_type, PaginationButtonType::PageNumber);
        assert_eq!(buttons[2].button_type, PaginationButtonType::Prev);
        assert!(buttons[2].is_disabled);
    }

    fn test_dom_element(
        index: u32,
        tag_name: &str,
        name: Option<&str>,
        attributes: &[(&str, &str)],
    ) -> DomElementRef {
        DomElementRef {
            index,
            target_id: "target".to_owned(),
            backend_node_id: u64::from(index),
            node_id: None,
            tag_name: tag_name.to_owned(),
            role: None,
            name: name.map(str::to_owned),
            text: None,
            attributes: attributes
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    #[test]
    fn finds_previous_navigation_history_entry() {
        let entry_id = previous_navigation_entry_id(&json!({
            "currentIndex": 2,
            "entries": [
                { "id": 10, "url": "https://example.com/one" },
                { "id": 11, "url": "https://example.com/two" },
                { "id": 12, "url": "https://example.com/three" }
            ]
        }))
        .expect("previous entry");

        assert_eq!(entry_id, 11);
    }

    #[test]
    fn reports_missing_previous_navigation_entry() {
        let error = previous_navigation_entry_id(&json!({
            "currentIndex": 0,
            "entries": [
                { "id": 10, "url": "https://example.com/one" }
            ]
        }))
        .expect_err("missing previous entry");

        assert!(matches!(error, BrowserError::ActionFailed(_)));
    }

    #[test]
    fn resolves_full_and_short_page_target_ids() {
        let tabs = vec![
            TabInfo {
                url: "https://example.com/one".to_owned(),
                title: "One".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-aaa111"),
                target_id: "target-aaa111".to_owned(),
                parent_target_id: None,
            },
            TabInfo {
                url: "https://example.com/two".to_owned(),
                title: "Two".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-bbb222"),
                target_id: "target-bbb222".to_owned(),
                parent_target_id: None,
            },
        ];

        assert_eq!(
            resolve_page_target_id_from_tabs(&tabs, "target-aaa111").expect("full target id"),
            "target-aaa111"
        );
        assert_eq!(
            resolve_page_target_id_from_tabs(&tabs, "b222").expect("short target id"),
            "target-bbb222"
        );
        assert!(matches!(
            resolve_page_target_id_from_tabs(&tabs, "nope"),
            Err(BrowserError::ActionFailed(_))
        ));
    }

    #[test]
    fn scroll_to_text_script_json_escapes_text() {
        let script = scroll_to_text_js(r#"Needle "quoted""#).expect("scroll script");

        assert!(script.contains(r#"Needle \"quoted\""#));
        assert!(script.contains("scrollIntoView"));
    }

    #[test]
    fn send_keys_normalizes_aliases_and_shortcuts() {
        assert_eq!(normalize_send_keys("ctrl+a"), "Control+a");
        assert_eq!(normalize_send_keys("Command+Shift+P"), "Meta+Shift+P");
        assert_eq!(normalize_send_keys("pagedown"), "PageDown");
        assert_eq!(normalize_send_keys("esc"), "Escape");
        assert_eq!(normalize_send_keys(" keep spaces "), " keep spaces ");
    }

    #[test]
    fn send_keys_key_events_include_codes_and_modifiers() {
        assert_eq!(
            modifier_mask(&["Control".to_owned(), "Shift".to_owned()]),
            10
        );

        assert_eq!(
            key_event_params("keyDown", "a", 2),
            json!({
                "type": "keyDown",
                "key": "a",
                "code": "KeyA",
                "modifiers": 2,
                "windowsVirtualKeyCode": 65,
            })
        );
        assert_eq!(
            key_event_params("keyUp", "PageDown", 0),
            json!({
                "type": "keyUp",
                "key": "PageDown",
                "code": "PageDown",
                "windowsVirtualKeyCode": 34,
            })
        );
    }

    #[test]
    fn dropdown_scripts_support_aria_options() {
        let options_script = dropdown_options_js(2);
        let select_script =
            select_dropdown_option_js(2, r#"Two "quoted""#).expect("select dropdown script");

        assert!(options_script.contains("aria-controls"));
        assert!(options_script.contains(r#"[role="option"]"#));
        assert!(options_script.contains("ARIA listbox"));
        assert!(select_script.contains(r#"const requested = "Two \"quoted\"";"#));
        assert!(select_script.contains("aria-selected"));
        assert!(select_script.contains("MouseEvent('click'"));
    }

    #[test]
    fn click_script_rejects_select_and_file_inputs() {
        let script = click_element_js(1);

        assert!(script.contains("Cannot click on <select> elements."));
        assert!(script.contains("select_dropdown_option"));
        assert!(script.contains("Cannot click on file input elements."));
        assert!(script.contains("Use upload_file instead."));
        assert!(script.contains("dispatchEvent(new MouseEvent('click'"));
    }

    #[test]
    fn cached_click_function_uses_same_guard_body() {
        let function = element_action_function_js(CLICK_ELEMENT_ACTION_JS);

        assert!(function.contains("const el = this;"));
        assert!(function.contains("Cannot click on <select> elements."));
        assert!(function.contains("Cannot click on file input elements."));
        assert!(function.contains("el.click();"));
        assert!(function.contains("dispatchEvent(new MouseEvent('click'"));
    }

    #[test]
    fn dropdown_scripts_can_run_as_cached_element_functions() {
        let options_function = element_function_js(DROPDOWN_OPTIONS_BODY_JS);
        let select_body =
            select_dropdown_option_body_js("Enterprise").expect("select dropdown body");
        let select_function = element_function_js(&select_body);

        assert!(options_function.contains("const el = this;"));
        assert!(options_function.contains("return JSON.stringify(options);"));
        assert!(select_function.contains("const requested = \"Enterprise\";"));
        assert!(select_function.contains("No dropdown option found"));
    }

    #[test]
    fn interactive_snapshot_uses_image_alt_text_sources() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("descendantAltText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'img[alt], svg[aria-label]'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'alt'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'aria-describedby'"));
    }

    #[test]
    fn interactive_snapshot_uses_selected_option_text() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("controlValueText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("selectedOptions"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("controlText || el.innerText"));
    }

    #[test]
    fn interactive_snapshot_summarizes_select_compound_options() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("selectCompoundComponents"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("compound_components"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Dropdown Toggle"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("count=${options.length}"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("format=${formatHint}"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("... ${options.length - 4} more options..."));
    }

    #[test]
    fn interactive_snapshot_preserves_automation_attributes() {
        for attribute in [
            "aria-controls",
            "aria-disabled",
            "aria-haspopup",
            "aria-live",
            "aria-owns",
            "aria-placeholder",
            "aria-required",
            "aria-valuemax",
            "autocomplete",
            "data-cy",
            "data-datepicker",
            "data-inputmask",
            "data-mask",
            "data-selenium",
            "data-state",
            "data-testid",
            "data-test",
            "data-qa",
            "data-value",
            "for",
            "itemscope",
            "itemprop",
            "lang",
            "inputmode",
            "max",
            "maxlength",
            "min",
            "minlength",
            "pattern",
            "readonly",
            "step",
            "uib-datepicker-popup",
        ] {
            assert!(
                INTERACTIVE_ELEMENTS_JS.contains(attribute),
                "missing attribute {attribute}"
            );
        }
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.value = controlText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.checked = String(el.checked)"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.selected = String(el.selected)"));
    }

    #[test]
    fn interactive_snapshot_keeps_hidden_file_inputs() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isFileInput"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("toLowerCase() === 'file'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("if (isFileInput(el)) return true;"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isFileInput"));
        assert!(action_script.contains("if (isFileInput(el)) return true;"));
    }

    #[test]
    fn interactive_snapshot_skips_decorative_svg_children() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isDecorativeSvgChild"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'path'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'tspan'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isDecorativeSvgChild"));
        assert!(action_script.contains("'circle'"));
    }

    #[test]
    fn interactive_snapshot_marks_elements_for_accessibility_join() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains(AX_REF_ATTRIBUTE));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("ax_ref: axRef"));
        assert!(CLEANUP_AX_REFS_JS.contains(AX_REF_ATTRIBUTE));
    }

    #[test]
    fn dom_snapshot_refs_map_to_backend_node_ids() {
        let snapshot = json!({
            "strings": [
                AX_REF_ATTRIBUTE,
                "browser-use-rs-1",
                "id",
                "native-button"
            ],
            "documents": [{
                "nodes": {
                    "backendNodeId": [41, 42],
                    "attributes": [
                        [],
                        [0, 1, 2, 3]
                    ]
                }
            }]
        });

        let refs = snapshot_backend_ids_by_ax_ref(&snapshot);

        assert_eq!(refs.get("browser-use-rs-1"), Some(&42));
    }

    #[test]
    fn accessibility_tree_nodes_map_by_backend_id() {
        let tree = json!({
            "nodes": [
                {
                    "backendDOMNodeId": 42,
                    "role": { "type": "role", "value": "button" },
                    "name": { "type": "computedString", "value": "Save settings" },
                    "properties": [
                        { "name": "expanded", "value": { "type": "boolean", "value": true } },
                        { "name": "valuenow", "value": { "type": "number", "value": 7 } },
                        { "name": "valuetext", "value": { "type": "string", "value": "Seven" } }
                    ]
                },
                {
                    "backendDOMNodeId": 43,
                    "ignored": true,
                    "role": { "type": "role", "value": "button" },
                    "name": { "type": "computedString", "value": "Ignored" }
                }
            ]
        });

        let nodes = accessibility_nodes_by_backend_id(&tree);
        let button = nodes.get(&42).expect("button ax node");

        assert_eq!(button.role.as_deref(), Some("button"));
        assert_eq!(button.name.as_deref(), Some("Save settings"));
        assert_eq!(
            button.properties.get("expanded").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            button.properties.get("valuenow").map(String::as_str),
            Some("7")
        );
        assert_eq!(
            button.properties.get("valuetext").map(String::as_str),
            Some("Seven")
        );
        assert!(!nodes.contains_key(&43));
    }

    #[test]
    fn dom_element_uses_accessibility_enrichment() {
        let accessibility = BTreeMap::from([(
            "browser-use-rs-1".to_owned(),
            AccessibilityNodeInfo {
                backend_node_id: 42,
                node_id: Some(84),
                role: Some("button".to_owned()),
                name: Some("Save settings".to_owned()),
                properties: BTreeMap::from([("expanded".to_owned(), "true".to_owned())]),
            },
        )]);
        let element = dom_element_from_value(
            "target-1",
            &json!({
                "index": 1,
                "tag_name": "button",
                "name": "DOM fallback",
                "text": "DOM fallback",
                "attributes": { "id": "native-button" },
                "ax_ref": "browser-use-rs-1",
                "is_visible": true,
                "is_interactive": true
            }),
            &accessibility,
        )
        .expect("dom element");

        assert_eq!(element.backend_node_id, 42);
        assert_eq!(element.node_id, Some(84));
        assert_eq!(element.role.as_deref(), Some("button"));
        assert_eq!(element.name.as_deref(), Some("Save settings"));
        assert_eq!(
            element.attributes.get("expanded").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn dom_state_parser_carries_page_stats() {
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "stats": {
                    "links": 1,
                    "iframes": 2,
                    "shadow_open": 1,
                    "shadow_closed": 0,
                    "scroll_containers": 3,
                    "images": 4,
                    "interactive_elements": 5,
                    "total_elements": 30,
                    "text_chars": 40
                },
                "elements": [{
                    "index": 1,
                    "tag_name": "a",
                    "name": "Docs",
                    "text": "Docs",
                    "attributes": { "href": "/docs" },
                    "is_visible": true,
                    "is_interactive": true
                }]
            }),
            &BTreeMap::new(),
        )
        .expect("dom state");

        assert_eq!(state.selector_map.len(), 1);
        assert_eq!(state.page_stats.links, 1);
        assert_eq!(state.page_stats.iframes, 2);
        assert_eq!(state.page_stats.shadow_open, 1);
        assert_eq!(state.page_stats.scroll_containers, 3);
        assert_eq!(state.page_stats.images, 4);
        assert_eq!(state.page_stats.interactive_elements, 5);
        assert_eq!(state.page_stats.total_elements, 30);
        assert_eq!(state.page_stats.text_chars, 40);
    }

    #[test]
    fn interactive_snapshot_detects_search_affordances() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasSearchIndicator"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("search-icon"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attr.name.startsWith('data-')"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasSearchIndicator"));
        assert!(action_script.contains("search-button"));
    }

    #[test]
    fn interactive_snapshot_detects_small_icon_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasIconSignal"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("rect.width < 10"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'data-action'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'aria-label'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasIconSignal"));
        assert!(action_script.contains("rect.height > 50"));
    }

    #[test]
    fn interactive_snapshot_detects_pointer_cursor_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasPointerCursor"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("cursor === 'pointer'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasPointerCursor"));
        assert!(action_script.contains("cursor === 'pointer'"));
    }

    #[test]
    fn interactive_snapshot_detects_static_handlers_and_listboxes() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[role=\"listbox\"]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[onmousedown]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[onkeydown]"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("[role=\"listbox\"]"));
        assert!(action_script.contains("[onmouseup]"));
        assert!(action_script.contains("[onkeyup]"));
    }

    #[test]
    fn interactive_snapshot_filters_occluded_elements() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isTopmostAtCenter"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elementFromPoint"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("root.host"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isTopmostAtCenter"));
        assert!(action_script.contains("elementFromPoint"));
    }

    #[test]
    fn interactive_snapshot_skips_browser_use_excluded_subtrees() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isBrowserUseExcluded"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("data-browser-use-exclude"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("data-browser-use-exclude-"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isBrowserUseExcluded"));
        assert!(action_script.contains("data-browser-use-exclude"));
        assert!(action_script.contains("data-browser-use-exclude-"));
    }

    #[test]
    fn interactive_snapshot_skips_non_content_dom_tags() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isNonContentTag"));
        for tag in ["style", "script", "head", "meta", "link", "title"] {
            assert!(
                INTERACTIVE_ELEMENTS_JS.contains(tag),
                "state walker missing {tag}"
            );
        }

        let action_script = click_element_js(1);
        assert!(action_script.contains("isNonContentTag"));
        for tag in ["style", "script", "head", "meta", "link", "title"] {
            assert!(
                action_script.contains(tag),
                "action fallback walker missing {tag}"
            );
        }
    }

    #[test]
    fn interactive_snapshot_prunes_contained_action_descendants() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isPropagatingActionContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isContainedByPropagatingActionContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shouldKeepContainedDescendant"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("containedByRect"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains(">= 0.99"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isPropagatingActionContainer"));
        assert!(action_script.contains("isContainedByPropagatingActionContainer"));
        assert!(action_script.contains("shouldKeepContainedDescendant"));
        assert!(action_script.contains("containedByRect"));
        assert!(action_script.contains(">= 0.99"));
    }

    #[test]
    fn interactive_snapshot_detects_javascript_click_listeners() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("getEventListeners"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasJsClickListener"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'pointerdown'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("document.querySelectorAll('*').length <= 10000"));

        let params = runtime_evaluate_params(INTERACTIVE_ELEMENTS_JS, true);
        assert_eq!(params["includeCommandLineAPI"], true);

        let params = runtime_evaluate_params("document.title", false);
        assert!(params.get("includeCommandLineAPI").is_none());
    }

    #[test]
    fn interactive_snapshot_collects_page_statistics() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("const stats = {"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shadow_open"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("interactive_elements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("total_elements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("text_chars"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("return {"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elements: elements.slice"));
    }

    #[test]
    fn interactive_snapshot_indexes_scrollable_containers_without_descendant_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shouldIndexScrollable"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasInteractiveDescendant"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isDropdownContainer"));

        let action_script = element_action_js(1, "el.scrollBy(0, el.clientHeight);");
        assert!(action_script.contains("shouldIndexScrollable"));
        assert!(action_script.contains("hasInteractiveDescendant"));
        assert!(action_script.contains("isDropdownContainer"));
    }

    #[test]
    fn renders_runtime_evaluate_values() {
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "string", "value": "EvalOps" }
            }))
            .expect("string result"),
            "EvalOps"
        );
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "number", "value": 42 }
            }))
            .expect("number result"),
            "42"
        );
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "undefined" }
            }))
            .expect("undefined result"),
            "undefined"
        );
    }

    #[test]
    fn renders_runtime_evaluate_exception_as_error() {
        let error = render_runtime_evaluate_result(&json!({
            "exceptionDetails": { "text": "Uncaught Error: boom" }
        }))
        .expect_err("exception");

        assert!(matches!(error, BrowserError::CommandFailed { .. }));
    }

    #[test]
    fn executable_resolution_prefers_explicit_path() {
        let current_exe = std::env::current_exe().expect("current exe");
        let resolved = resolve_chrome_executable(Some(&current_exe), None, Vec::<PathBuf>::new())
            .expect("resolve executable");

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn executable_resolution_reports_checked_paths() {
        let missing = PathBuf::from("/definitely/not/a/chrome");
        let error = resolve_chrome_executable(None, None, vec![missing.clone()])
            .expect_err("missing executable");

        match error {
            BrowserError::ExecutableNotFound(checked) => assert_eq!(checked, vec![missing]),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn waits_for_devtools_endpoint_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let active_port_path = devtools_active_port_path(temp_dir.path());
        tokio::fs::write(&active_port_path, "38119\n/devtools/browser/abc123\n")
            .await
            .expect("write endpoint");

        let endpoint = wait_for_devtools_endpoint(temp_dir.path(), 100)
            .await
            .expect("endpoint");

        assert_eq!(endpoint.http_url, "http://127.0.0.1:38119");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn launches_local_chrome_when_available() {
        let profile = BrowserProfile::default();
        let browser = profile.launch_local().await.expect("launch local browser");

        assert!(browser.process_id().is_some());
        assert!(browser.endpoint().http_url.starts_with("http://127.0.0.1:"));
        assert!(
            browser
                .endpoint()
                .websocket_url
                .starts_with("ws://127.0.0.1:")
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_index_open_shadow_dom_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>shadow smoke</title></head><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'open'});const button=document.createElement('button');button.textContent='Shadow click';button.onclick=()=>{document.title='shadow clicked'};const input=document.createElement('input');input.placeholder='Shadow name';root.append(button,input);</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 2);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Shadow click")
        );

        session.click(1).await.expect("shadow click");
        session
            .input_text(2, "EvalOps", true)
            .await
            .expect("shadow input");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("shadow state");
        assert_eq!(state.title, "shadow clicked");
        assert!(
            state.dom_state.llm_representation().contains("EvalOps"),
            "DOM state did not include shadow input value: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_javascript_listener_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>listener smoke</title></head><body><div id='plain-listener' style='display:inline-block;width:80px;height:30px'>Plain listener</div><script>document.getElementById('plain-listener').addEventListener('click',()=>{document.title='listener clicked'});</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        let listener = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element.attributes.get("id").map(String::as_str) == Some("plain-listener")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing JS listener element: {}",
                    initial_state.dom_state.llm_representation()
                )
            });

        session
            .click(listener.index)
            .await
            .expect("listener-backed click");
        sleep(Duration::from_millis(100)).await;
        let state = session.state(false).await.expect("post-click state");

        assert_eq!(state.title, "listener clicked");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_index_same_origin_iframe_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>iframe smoke</title></head><body><script>const iframe=document.createElement('iframe');iframe.srcdoc='<button onclick=\"parent.document.title=&quot;iframe clicked&quot;\">Frame click</button><input placeholder=\"Frame name\">';document.body.appendChild(iframe);</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(200)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 3);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Frame click")
        );
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        assert_eq!(iframe.index, 1);
        let frame_button_index = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Frame click"))
            .expect("iframe button")
            .index;
        let frame_input_index = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Frame name"))
            .expect("iframe input")
            .index;

        session
            .click(frame_button_index)
            .await
            .expect("iframe click");
        session
            .input_text(frame_input_index, "EvalOps", true)
            .await
            .expect("iframe input");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("iframe state");
        assert_eq!(state.title, "iframe clicked");
        let iframe_input_value = session
            .evaluate_json(
                "document.querySelector('iframe').contentDocument.querySelector('input').value",
            )
            .await
            .expect("iframe input value");
        assert_eq!(iframe_input_value.as_str(), Some("EvalOps"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_labels_for_form_control_names() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>label smoke</title></head><body><label for='email'>Email address</label><input id='email' placeholder='Placeholder only'><span id='submit-name'>Submit request</span><button aria-labelledby='submit-name'>Ignored text</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let input = state.dom_state.selector_map.get(&1).expect("labeled input");
        assert_eq!(input.name.as_deref(), Some("Email address"));
        let button = state
            .dom_state
            .selector_map
            .get(&2)
            .expect("labelled button");
        assert_eq!(button.name.as_deref(), Some("Submit request"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_enriches_dom_from_accessibility_tree() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>ax smoke</title></head><body><button id='native-button'>Save settings</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let button = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "native-button")
            })
            .expect("native button");

        assert!(button.backend_node_id > 0);
        assert!(button.node_id.is_some_and(|node_id| node_id > 0));
        assert_eq!(button.role.as_deref(), Some("button"));
        assert_eq!(button.name.as_deref(), Some("Save settings"));

        let leaked_probe = session
            .evaluate_json(&format!(
                "document.querySelector('[{}]') !== null",
                AX_REF_ATTRIBUTE
            ))
            .await
            .expect("probe leak check");
        assert_eq!(leaked_probe.as_bool(), Some(false));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_click_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable click smoke</title></head><body><button id='target' onclick=\"document.title='target clicked'\">Target</button><script>function insertBeforeTarget(){const button=document.createElement('button');button.id='inserted';button.textContent='Inserted';button.onclick=()=>{document.title='inserted clicked'};document.body.insertBefore(button, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target button")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert button before observed target");
        session
            .click(target_index)
            .await
            .expect("click cached target");

        let title = session
            .evaluate_json("document.title")
            .await
            .expect("title");
        assert_eq!(title.as_str(), Some("target clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_input_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable input smoke</title></head><body><input id='target' placeholder='Target'><script>function insertBeforeTarget(){const input=document.createElement('input');input.id='inserted';input.placeholder='Inserted';document.body.insertBefore(input, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target input")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert input before observed target");
        session
            .input_text(target_index, "EvalOps", true)
            .await
            .expect("input cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').value, inserted: document.getElementById('inserted').value })",
            )
            .await
            .expect("values");
        let values: Value =
            serde_json::from_str(values.as_str().expect("encoded values")).expect("values json");
        assert_eq!(values["target"].as_str(), Some("EvalOps"));
        assert_eq!(values["inserted"].as_str(), Some(""));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_scroll_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable scroll smoke</title></head><body><div id='target' tabindex='0' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Target pane</div></div><script>function insertBeforeTarget(){const pane=document.createElement('div');pane.id='inserted';pane.tabIndex=0;pane.style.cssText='height:60px;width:200px;overflow:auto;border:1px solid black';const inner=document.createElement('div');inner.style.height='400px';inner.textContent='Inserted pane';pane.appendChild(inner);document.body.insertBefore(pane, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target pane")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert pane before observed target");
        session
            .scroll(Some(target_index), true, 1.0)
            .await
            .expect("scroll cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').scrollTop, inserted: document.getElementById('inserted').scrollTop })",
            )
            .await
            .expect("scroll values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded scroll values"))
            .expect("scroll values json");
        assert!(values["target"].as_f64().unwrap_or_default() > 0.0);
        assert_eq!(values["inserted"].as_f64(), Some(0.0));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_dropdown_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable dropdown smoke</title></head><body><select id='target'><option>Starter</option><option>Enterprise</option></select><script>function insertBeforeTarget(){const select=document.createElement('select');select.id='inserted';select.innerHTML='<option>Inserted A</option><option>Inserted B</option>';document.body.insertBefore(select, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target select")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert select before observed target");
        let options = session
            .dropdown_options(target_index)
            .await
            .expect("cached target options");
        assert_eq!(options, ["Starter", "Enterprise"]);

        session
            .select_dropdown_option(target_index, "Enterprise")
            .await
            .expect("select cached target option");
        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').value, inserted: document.getElementById('inserted').value })",
            )
            .await
            .expect("select values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded select values"))
            .expect("select values json");
        assert_eq!(values["target"].as_str(), Some("Enterprise"));
        assert_eq!(values["inserted"].as_str(), Some("Inserted A"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_upload_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("cached-upload.txt");
        std::fs::write(&upload_path, "EvalOps cached upload").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>stable upload smoke</title></head><body><input id='target' type='file'><script>function insertBeforeTarget(){const input=document.createElement('input');input.id='inserted';input.type='file';document.body.insertBefore(input, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target file input")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert file input before observed target");
        session
            .upload_file(target_index, &upload_path)
            .await
            .expect("upload cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').files[0]?.name || '', inserted: document.getElementById('inserted').files[0]?.name || '' })",
            )
            .await
            .expect("upload values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded upload values"))
            .expect("upload values json");
        assert_eq!(values["target"].as_str(), Some("cached-upload.txt"));
        assert_eq!(values["inserted"].as_str(), Some(""));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_hidden_file_inputs_for_upload() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("hidden-upload.txt");
        std::fs::write(&upload_path, "EvalOps hidden upload").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>hidden upload smoke</title></head><body><label for='hidden-file'>Upload</label><input id='hidden-file' type='file' style='display:none' onchange=\"document.body.dataset.uploaded=this.files[0]?.name || ''\"></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let hidden_file = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "hidden-file")
            })
            .expect("hidden file input indexed");
        assert_eq!(hidden_file.tag_name, "input");
        assert_eq!(
            hidden_file.attributes.get("type").map(String::as_str),
            Some("file")
        );

        session
            .upload_file(hidden_file.index, &upload_path)
            .await
            .expect("upload hidden file input");
        let uploaded_name = session
            .evaluate_json("document.body.dataset.uploaded || ''")
            .await
            .expect("uploaded file name");
        assert_eq!(uploaded_name.as_str(), Some("hidden-upload.txt"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_image_alt_for_control_names() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>alt smoke</title></head><body><a id='report' href='https://example.com/report'><img alt='Download report' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==' style='width:24px;height:24px'></a><button id='settings'><img alt='Open settings' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==' style='width:24px;height:24px'></button><input id='image-submit' type='image' alt='Search icon' style='width:24px;height:24px'></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        assert_eq!(
            element_by_id("report").name.as_deref(),
            Some("Download report")
        );
        assert_eq!(
            element_by_id("settings").name.as_deref(),
            Some("Open settings")
        );
        assert_eq!(
            element_by_id("image-submit").name.as_deref(),
            Some("Search icon")
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_decorative_svg_children() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>svg smoke</title></head><body><svg id='svg-button' role='button' aria-label='Open vector' onclick=\"document.title='svg clicked'\" width='32' height='32'><path id='decorative-path' onclick=\"document.title='path clicked'\" d='M0 0h32v32H0z'></path></svg></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let svg = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "svg-button")
            })
            .expect("svg root indexed");
        assert_eq!(svg.tag_name, "svg");
        assert_eq!(svg.role.as_deref(), Some("button"));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "decorative-path")
        }));

        session.click(svg.index).await.expect("click svg by index");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("svg clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_filters_occluded_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>occlusion smoke</title></head><body><button id='covered' onclick=\"document.title='covered clicked'\" style='position:absolute;left:20px;top:20px;width:120px;height:40px'>Covered</button><div id='cover' style='position:absolute;left:0;top:0;width:220px;height:100px;background:white;z-index:2'></div><button id='visible' onclick=\"document.title='visible clicked'\" style='position:absolute;left:20px;top:140px;width:120px;height:40px'>Visible</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"visible"), "missing visible button: {ids:?}");
        assert!(
            !ids.contains(&"covered"),
            "covered button should not be indexed: {ids:?}"
        );

        let visible = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "visible")
            })
            .expect("visible button indexed");

        session.click(visible.index).await.expect("click visible");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("visible clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_selected_option_as_select_text() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>select smoke</title></head><body><label for='plan'>Plan</label><select id='plan'><option>Starter</option><option selected>Enterprise</option><option>Internal</option></select></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let select = state.dom_state.selector_map.get(&1).expect("select");
        assert_eq!(select.name.as_deref(), Some("Plan"));
        assert_eq!(select.text.as_deref(), Some("Enterprise"));
        let compound_components = select
            .attributes
            .get("compound_components")
            .expect("select compound components");
        assert!(compound_components.contains("Dropdown Toggle"));
        assert!(compound_components.contains("count=3"));
        assert!(compound_components.contains("options=Starter|Enterprise|Internal"));
        assert!(
            state
                .dom_state
                .llm_representation()
                .contains("Plan Enterprise"),
            "DOM state did not include selected option value: {}",
            state.dom_state.llm_representation()
        );
        assert!(
            !state.dom_state.llm_representation().contains("Starter"),
            "DOM state included unselected option text: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_accessibility_state_properties() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>ax props smoke</title></head><body><button id='toggle' aria-expanded='true'>Details</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        let llm = state.dom_state.llm_representation();
        assert!(
            llm.contains("expanded=true"),
            "DOM state did not include AX expanded property: {llm}"
        );
        assert!(
            !llm.contains("aria-expanded=true"),
            "DOM state did not prefer AX expanded over aria-expanded: {llm}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_detects_pagination_buttons() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>pagination smoke</title></head><body><nav><button id='previous' class='disabled'>Previous</button><a id='page-two' href='https://example.com/page/2'>2</a><button id='next'>Next</button><button id='export'>Export</button></nav></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        assert_eq!(state.pagination_buttons.len(), 3);
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::Prev
                && button.text.contains("Previous")
                && button.is_disabled
        }));
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::Next && button.selector == "#next"
        }));
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::PageNumber && button.text == "2"
        }));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_accessibility_widget_roles() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>roles smoke</title></head><body><details id='details'><summary id='summary'>More details</summary><p>Body</p></details><div id='menuitem' role='menuitem' aria-label='Open menu'>Menu</div><div id='checkbox' role='checkbox' aria-checked='false'>Subscribe</div><div id='hidden-role' role='button' aria-hidden='true'>Hidden role</div><button id='disabled-button' disabled>Disabled</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        let summary = element_by_id("summary");
        assert_eq!(summary.tag_name, "summary");
        assert_eq!(summary.name.as_deref(), Some("More details"));

        let menuitem = element_by_id("menuitem");
        assert_eq!(menuitem.role.as_deref(), Some("menuitem"));
        assert_eq!(menuitem.name.as_deref(), Some("Open menu"));

        let checkbox = element_by_id("checkbox");
        assert_eq!(checkbox.role.as_deref(), Some("checkbox"));
        assert_eq!(checkbox.name.as_deref(), Some("Subscribe"));

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "hidden-role" && id != "disabled-button")
        }));

        session
            .click(summary.index)
            .await
            .expect("click summary element");
        let details_open = session
            .evaluate_json("document.getElementById('details').open")
            .await
            .expect("details open");
        assert_eq!(details_open.as_bool(), Some(true));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_browser_use_excluded_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>exclude smoke</title></head><body><button id='visible' onclick=\"document.title='visible clicked'\">Visible</button><button id='legacy' data-browser-use-exclude='true'>Legacy</button><div id='scoped' data-browser-use-exclude-demo='TRUE'><button id='nested'>Nested</button></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let visible = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "visible")
            })
            .expect("visible button indexed");

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "legacy" && id != "scoped" && id != "nested")
        }));

        session
            .click(visible.index)
            .await
            .expect("click visible element by index");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("visible clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_search_affordance_signals() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>search smoke</title></head><body><div id='site-search' class='search-icon' style='width:24px;height:24px'>Find</div><div data-action='open-search' style='width:24px;height:24px'>Lookup</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let search = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "site-search")
            })
            .expect("search affordance indexed");
        assert_eq!(search.tag_name, "div");
        assert_eq!(search.name.as_deref(), Some("Find"));

        session
            .click(search.index)
            .await
            .expect("click search affordance by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_small_icon_controls() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>icon smoke</title></head><body><span id='favorite-icon' data-action='favorite' aria-label='Favorite' style='display:inline-block;width:24px;height:24px'></span><span id='plain-small' style='display:inline-block;width:24px;height:24px'></span></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let favorite = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "favorite-icon")
            })
            .expect("icon control indexed");
        assert_eq!(favorite.tag_name, "span");
        assert_eq!(favorite.name.as_deref(), Some("Favorite"));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "plain-small")
        }));

        session
            .click(favorite.index)
            .await
            .expect("click icon control by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_pointer_cursor_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>pointer cursor</title></head><body><div id='pointer' style='cursor:pointer;width:120px;height:32px'>Pointer target</div><div id='plain' style='width:120px;height:32px'>Plain target</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();
        assert!(
            ids.contains(&"pointer"),
            "DOM state did not index pointer cursor control: {}",
            state.dom_state.llm_representation()
        );
        assert!(
            !ids.contains(&"plain"),
            "plain non-pointer div should not be indexed: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_static_handlers_and_listboxes() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>static handler</title></head><body><div id='choices' role='listbox' style='width:160px;height:32px'>Choices</div><div id='mouse-down' onmousedown='document.body.dataset.mouse=\"down\"' style='width:120px;height:32px'>Mouse down</div><div id='key-down' onkeydown='document.body.dataset.key=\"down\"' style='width:120px;height:32px'>Key down</div><div id='plain-static' style='width:120px;height:32px'>Plain static</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();
        for expected in ["choices", "mouse-down", "key-down"] {
            assert!(
                ids.contains(&expected),
                "DOM state did not index {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        assert!(
            !ids.contains(&"plain-static"),
            "plain static div should not be indexed: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_blocks_disallowed_profile_navigation() {
        let profile = BrowserProfile {
            allowed_domains: vec!["example.com".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        let error = session
            .navigate("https://blocked.test", false)
            .await
            .expect_err("disallowed navigation should be blocked before CDP navigation");

        assert!(matches!(
            error,
            BrowserError::NavigationBlocked { ref reason, .. } if reason == "not_in_allowed_domains"
        ));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_resets_disallowed_redirect_after_navigation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect server");
        let server_addr = listener.local_addr().expect("redirect server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept redirect request");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write redirect response");
        });
        let start_url = format!("http://localhost:{}/start", server_addr.port());

        let error = session
            .navigate(&start_url, false)
            .await
            .expect_err("redirected navigation should be reset by URL policy");
        server.await.expect("redirect server task");

        assert!(
            matches!(error, BrowserError::NavigationBlocked { .. }),
            "unexpected redirect policy error: {error:?}"
        );

        sleep(Duration::from_millis(250)).await;
        let state = session.state(false).await.expect("state after reset");
        assert_eq!(state.url, "about:blank");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_closes_disallowed_unsolicited_new_tab() {
        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let blocked_target_id = create_target(&session.connection, "http://127.0.0.1:1/popup")
            .await
            .expect("create blocked tab");

        sleep(Duration::from_millis(250)).await;
        let error = session
            .state(false)
            .await
            .expect_err("state observation should close the blocked tab");

        assert!(matches!(
            error,
            BrowserError::NavigationBlocked { ref url, ref reason }
                if url.starts_with("http://127.0.0.1:1/popup")
                    && reason == "ip_address_blocked"
        ));

        sleep(Duration::from_millis(150)).await;
        let tabs = page_tabs(&session.connection)
            .await
            .expect("tabs after policy enforcement");
        assert!(
            tabs.iter().all(|tab| tab.target_id != blocked_target_id),
            "blocked tab still open: {tabs:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_rejects_disallowed_new_tab_from_click_action() {
        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>blocked click</title></head><body><a id='blocked' target='_blank' href='http://127.0.0.1:1/popup'>Blocked popup</a></body></html>",
                false,
            )
            .await
            .expect("navigate allowed data page");
        sleep(Duration::from_millis(100)).await;
        let state = session.state(false).await.expect("initial state");
        let index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|id| id == "blocked")
            })
            .expect("blocked link")
            .index;

        let error = session
            .click(index)
            .await
            .expect_err("click should enforce blocked popup policy");

        assert!(matches!(
            error,
            BrowserError::NavigationBlocked { ref url, ref reason }
                if url.starts_with("http://127.0.0.1:1/popup")
                    && reason == "ip_address_blocked"
        ));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_scrolls_indexed_scrollable_element() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>scrollable smoke</title></head><body><button style='display:none'>Hidden</button><div id='pane' tabindex='0' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Top<br><button>Deep button</button></div></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        assert!(!state.dom_state.llm_representation().contains("Hidden"));
        let pane = state
            .dom_state
            .selector_map
            .get(&1)
            .expect("scrollable pane");
        assert!(
            pane.is_scrollable,
            "pane was not marked scrollable: {pane:?}"
        );

        session
            .scroll(Some(1), true, 1.0)
            .await
            .expect("scroll pane");
        let scroll_top = session
            .evaluate_json("document.getElementById('pane').scrollTop")
            .await
            .expect("scrollTop");
        assert!(scroll_top.as_f64().unwrap_or_default() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_plain_scroll_container_without_tabindex() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>plain scroll container</title></head><body><div id='plain-pane' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Plain scroll content</div></div><div id='button-pane' style='height:60px;width:200px;overflow:auto;border:1px solid black'><button id='inner-button'>Inner button</button><div style='height:400px'></div></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        let plain_pane = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "plain-pane")
            })
            .expect("plain scroll pane indexed");
        assert!(
            plain_pane.is_scrollable,
            "plain pane was not marked scrollable: {plain_pane:?}"
        );
        assert!(state.dom_state.selector_map.values().any(|element| {
            element
                .attributes
                .get("id")
                .is_some_and(|id| id == "inner-button")
        }));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "button-pane")
        }));

        session
            .scroll(Some(plain_pane.index), true, 1.0)
            .await
            .expect("scroll plain pane");
        let scroll_top = session
            .evaluate_json("document.getElementById('plain-pane').scrollTop")
            .await
            .expect("scrollTop");
        assert!(scroll_top.as_f64().unwrap_or_default() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_non_content_dom_tags() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>Hidden title copy</title><meta name='hidden' content='Hidden meta copy'><link rel='stylesheet' href='data:text/css,button{}'><style>Hidden style copy</style><script>window.__hiddenScriptCopy='Hidden script copy';</script></head><body><button id='visible' onclick=\"document.body.dataset.clicked='true'\">Visible</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        assert_eq!(state.dom_state.element_count(), 1);
        assert_eq!(state.dom_state.page_stats.total_elements, 3);
        assert_eq!(state.dom_state.page_stats.text_chars, 7);
        assert_eq!(
            state
                .dom_state
                .selector_map
                .values()
                .next()
                .and_then(|element| element.attributes.get("id"))
                .map(String::as_str),
            Some("visible")
        );
        for hidden_text in [
            "Hidden title copy",
            "Hidden style copy",
            "Hidden script copy",
        ] {
            assert!(
                !state.dom_state.llm_representation().contains(hidden_text),
                "non-content text leaked into DOM state: {}",
                state.dom_state.llm_representation()
            );
        }

        session.click(1).await.expect("click visible button");
        let clicked = session
            .evaluate_json("document.body.dataset.clicked")
            .await
            .expect("clicked flag");
        assert_eq!(clicked.as_str(), Some("true"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_prunes_contained_action_descendants() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>contained descendants</title></head><body><button id='outer-button' onclick=\"document.body.dataset.clicked='outer'\" style='width:160px;height:44px'><span id='button-icon' class='icon' style='display:inline-block;width:20px;height:20px'>x</span>Open</button><a id='outer-link' href='https://example.com/docs' style='display:inline-block;width:160px;height:44px'><span id='link-icon' class='icon' style='display:inline-block;width:20px;height:20px'>x</span>Docs</a><button id='labelled-outer' style='width:160px;height:44px'><span id='labelled-child' aria-label='Inner dismiss' style='display:inline-block;width:20px;height:20px'>x</span></button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();

        for expected in [
            "outer-button",
            "outer-link",
            "labelled-outer",
            "labelled-child",
        ] {
            assert!(
                ids.contains(&expected),
                "DOM state missing {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        for pruned in ["button-icon", "link-icon"] {
            assert!(
                !ids.contains(&pruned),
                "contained generic descendant should be pruned: {pruned}; ids={ids:?}"
            );
        }

        let outer_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|id| id == "outer-button")
            })
            .map(|element| element.index)
            .expect("outer button index");
        session
            .click(outer_index)
            .await
            .expect("click outer button by index");
        let clicked = session
            .evaluate_json("document.body.dataset.clicked")
            .await
            .expect("clicked flag");
        assert_eq!(clicked.as_str(), Some("outer"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_navigate_read_state_and_capture_screenshot() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("sample-upload.txt");
        std::fs::write(&upload_path, "EvalOps upload smoke").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>browser-use-rs smoke</title></head><body><button onclick=\"document.title='clicked'\">Click me</button><input placeholder='Name'><input type='file' onchange=\"document.body.dataset.uploaded=this.files[0]?.name || ''\"><div style='height:2000px'>Scroll target</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 3);
        assert!(initial_state.dom_state.page_stats.total_elements >= 5);
        assert_eq!(initial_state.dom_state.page_stats.interactive_elements, 3);
        assert!(initial_state.dom_state.page_stats.text_chars > 0);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Click me")
        );

        session.click(1).await.expect("click by index");
        sleep(Duration::from_millis(100)).await;
        session
            .input_text(2, "EvalOps", true)
            .await
            .expect("input text");
        session
            .click_coordinates(20, 20)
            .await
            .expect("coordinate click");
        session
            .upload_file(3, &upload_path)
            .await
            .expect("upload file");
        let uploaded_name = session
            .evaluate_json("document.body.dataset.uploaded || ''")
            .await
            .expect("uploaded file name");
        assert_eq!(uploaded_name.as_str(), Some("sample-upload.txt"));
        session.scroll(None, true, 0.25).await.expect("scroll");

        let state = session.state(true).await.expect("state");

        assert!(state.url.starts_with("data:text/html"));
        assert_eq!(state.title, "clicked");
        assert!(
            state.dom_state.llm_representation().contains("EvalOps"),
            "DOM state did not include typed input value: {}",
            state.dom_state.llm_representation()
        );
        assert!(state.screenshot.expect("screenshot").len() > 100);

        let original_target_id = state.tabs.first().expect("original tab").target_id.clone();
        session
            .navigate(
                "data:text/html,<html><head><title>browser-use-rs tab smoke</title></head><body>Second tab</body></html>",
                true,
            )
            .await
            .expect("navigate new tab");
        sleep(Duration::from_millis(100)).await;

        let tab_state = session.state(false).await.expect("new tab state");
        assert_eq!(tab_state.title, "browser-use-rs tab smoke");
        assert!(tab_state.tabs.len() >= 2);
        let new_target_id = tab_state
            .tabs
            .iter()
            .find(|tab| tab.title == "browser-use-rs tab smoke")
            .expect("new tab target")
            .target_id
            .clone();

        session
            .switch_tab(&original_target_id)
            .await
            .expect("switch original tab");
        sleep(Duration::from_millis(100)).await;
        let switched_state = session.state(false).await.expect("switched state");
        assert_eq!(switched_state.title, "clicked");

        session
            .switch_tab(&new_target_id)
            .await
            .expect("switch new tab");
        session
            .close_tab(&new_target_id)
            .await
            .expect("close new tab");
        sleep(Duration::from_millis(100)).await;

        let after_close = session.state(false).await.expect("state after close");
        assert_eq!(after_close.title, "clicked");
        assert!(
            after_close
                .tabs
                .iter()
                .all(|tab| tab.target_id != new_target_id)
        );
    }
}
