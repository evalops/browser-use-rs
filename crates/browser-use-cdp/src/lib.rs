//! Chrome DevTools Protocol browser-session layer.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use browser_use_dom::{
    BrowserStateSummary, DomElementRef, DomEvalNode, DomEvalNodeType, DomPageStats, ElementBounds,
    PageInfo, PaginationButton, PaginationButtonType, SerializedDomState, TabInfo,
    render_element_text,
};
use futures_util::{SinkExt, StreamExt};
use percent_encoding::percent_decode_str;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use tempfile::TempDir;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use unicode_normalization::UnicodeNormalization;

const AX_REF_ATTRIBUTE: &str = "data-browser-use-rs-ax-ref";
const URL_POLICY_SETTLE_MS: u64 = 200;
const MAX_SECURITY_EVENTS: usize = 8;
const MAX_LIFECYCLE_EVENTS: usize = 32;
const CDP_RECONNECT_MAX_ATTEMPTS: u32 = 3;
const CDP_RECONNECT_DELAYS_MS: [u64; 3] = [1_000, 2_000, 4_000];
const CDP_CONNECT_TIMEOUT_MS: u64 = 15_000;
const CLOUD_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

const INTERACTIVE_ELEMENTS_JS: &str = r#"
(() => {
  const axRefAttribute = 'data-browser-use-rs-ax-ref';
  const maxIframeDepth = 5;
  const maxIframeDocuments = 100;
  const paintOrderFiltering = true;
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
  const hasAriaInteractivityProperty = (el) => {
    const required = String(el.getAttribute('aria-required') || '').toLowerCase();
    if (required === 'true') return true;
    const autocomplete = String(el.getAttribute('aria-autocomplete') || '').toLowerCase();
    if (autocomplete && autocomplete !== 'none') return true;
    return String(el.getAttribute('aria-keyshortcuts') || '').trim().length > 0;
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
    return !isDisabledOrHidden(el) && rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden' && (!paintOrderFiltering || isTopmostAtCenter(el));
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
    if (hasAriaInteractivityProperty(el)) return true;
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
  const scrollInfoText = (el) => {
    if (!isScrollable(el)) return '';
    const visibleHeight = el.clientHeight || 0;
    const visibleWidth = el.clientWidth || 0;
    const scrollableHeight = el.scrollHeight || 0;
    const scrollableWidth = el.scrollWidth || 0;
    const parts = [];
    if (visibleHeight > 0 && scrollableHeight > visibleHeight + 1) {
      const pagesAbove = Math.max(0, el.scrollTop || 0) / visibleHeight;
      const pagesBelow = Math.max(0, scrollableHeight - visibleHeight - (el.scrollTop || 0)) / visibleHeight;
      parts.push(`${pagesAbove.toFixed(1)} pages above, ${pagesBelow.toFixed(1)} pages below`);
    }
    if (visibleWidth > 0 && scrollableWidth > visibleWidth + 1) {
      const maxScrollLeft = Math.max(1, scrollableWidth - visibleWidth);
      const pct = Math.round((Math.max(0, el.scrollLeft || 0) / maxScrollLeft) * 100);
      parts.push(`horizontal ${pct}%`);
    }
    return parts.join(' ');
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
  const inputCompoundComponents = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag !== 'input') return '';
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (['date', 'time', 'datetime-local', 'month', 'week'].includes(type)) return '';
    if (type === 'range') {
      const min = el.getAttribute('min') || '0';
      const max = el.getAttribute('max') || '100';
      return `(name=Value,role=slider,min=${min},max=${max})`;
    }
    if (type === 'number') {
      const min = el.getAttribute('min');
      const max = el.getAttribute('max');
      const valueParts = ['name=Value', 'role=textbox'];
      if (min) valueParts.push(`min=${min}`);
      if (max) valueParts.push(`max=${max}`);
      return `(name=Increment,role=button),(name=Decrement,role=button),(${valueParts.join(',')})`;
    }
    if (type === 'color') {
      return '(name=Hex Value,role=textbox),(name=Color Picker,role=button)';
    }
    if (type === 'file') {
      const current = Array.from(el.files || []).map((file) => file.name).filter(Boolean).join('|') || 'None';
      const selectedName = el.multiple ? 'Files Selected' : 'File Selected';
      return `(name=Browse Files,role=button),(name=${selectedName},role=textbox,current=${current})`;
    }
    return '';
  };
  const staticCompoundComponents = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag === 'details') {
      return '(name=Toggle Disclosure,role=button),(name=Content Area,role=region)';
    }
    if (tag === 'audio') {
      return '(name=Play/Pause,role=button),(name=Progress,role=slider,min=0,max=100),(name=Mute,role=button),(name=Volume,role=slider,min=0,max=100)';
    }
    if (tag === 'video') {
      return '(name=Play/Pause,role=button),(name=Progress,role=slider,min=0,max=100),(name=Mute,role=button),(name=Volume,role=slider,min=0,max=100),(name=Fullscreen,role=button)';
    }
    return '';
  };
  const compoundComponentsFor = (el) => {
    return selectCompoundComponents(el) || inputCompoundComponents(el) || staticCompoundComponents(el);
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
  let visitedIframeDocuments = 0;
  const visitFrame = (iframe, offset, depth) => {
    if (!isVisible(iframe)) return;
    if (depth >= maxIframeDepth) return;
    if (visitedIframeDocuments >= maxIframeDocuments) return;
    try {
      const frameDocument = iframe.contentDocument;
      if (!frameDocument) return;
      visitedIframeDocuments += 1;
      const rect = iframe.getBoundingClientRect();
      visitChildren(frameDocument, { x: offset.x + rect.x, y: offset.y + rect.y }, depth + 1);
    } catch (_) {
      return;
    }
  };
  const visitNode = (node, offset, depth) => {
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
      visitChildren(node.shadowRoot, offset, depth);
    }
    if (tag === 'iframe' || tag === 'frame') visitFrame(node, offset, depth);
    visitChildren(node, offset, depth);
  };
  const visitChildren = (root, offset, depth) => {
    for (const child of Array.from(root.children || [])) visitNode(child, offset, depth);
  };
  visitChildren(document, { x: 0, y: 0 }, 0);
  const booleanAttributeNames = new Set(['checked', 'disabled', 'multiple', 'readonly', 'required', 'selected']);
  const snapshotAttributeNames = ['id', 'class', 'name', 'type', 'placeholder', 'value', 'href', 'src', 'alt', 'aria-label', 'aria-labelledby', 'aria-describedby', 'aria-atomic', 'aria-autocomplete', 'aria-busy', 'aria-checked', 'aria-controls', 'aria-current', 'aria-disabled', 'aria-expanded', 'aria-haspopup', 'aria-hidden', 'aria-invalid', 'aria-keyshortcuts', 'aria-level', 'aria-live', 'aria-multiselectable', 'aria-owns', 'aria-placeholder', 'aria-pressed', 'aria-readonly', 'aria-required', 'aria-selected', 'aria-valuemax', 'aria-valuemin', 'aria-valuenow', 'aria-valuetext', 'role', 'title', 'contenteditable', 'data-cy', 'data-selenium', 'data-test', 'data-testid', 'data-qa', 'data-state', 'data-value', 'data-mask', 'data-inputmask', 'data-datepicker', 'data-date-format', 'uib-datepicker-popup', 'for', 'required', 'disabled', 'readonly', 'selected', 'multiple', 'accept', 'target', 'rel', 'list', 'tabindex', 'inputmode', 'autocomplete', 'pattern', 'min', 'max', 'minlength', 'maxlength', 'step', 'lang', 'itemscope', 'itemtype', 'itemprop', 'pseudo'];
  const evalAttributeNames = ['id', 'class', 'name', 'type', 'placeholder', 'aria-label', 'role', 'value', 'data-testid', 'alt', 'title', 'checked', 'selected', 'disabled', 'required', 'readonly', 'aria-expanded', 'aria-pressed', 'aria-checked', 'aria-selected', 'aria-invalid', 'pattern', 'min', 'max', 'minlength', 'maxlength', 'step', 'aria-valuemin', 'aria-valuemax', 'aria-valuenow'];
  const collectAttributes = (el, names) => {
    const attrs = {};
    for (const name of names) {
      const value = el.getAttribute(name);
      if (value !== null && value !== '') attrs[name] = value;
      else if (value === '' && booleanAttributeNames.has(name)) attrs[name] = 'true';
    }
    return attrs;
  };
  const indexedElements = elements.slice(0, 400).map(({ el, offset }, index) => {
    const rect = el.getBoundingClientRect();
    const axRef = `browser-use-rs-${index + 1}`;
    try { el.setAttribute(axRefAttribute, axRef); } catch (_) {}
    const attrs = collectAttributes(el, snapshotAttributeNames);
    const altText = descendantAltText(el);
    const controlText = controlValueText(el);
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (controlText && type !== 'password') attrs.value = controlText;
    if ((tag === 'input' || tag === 'option') && 'checked' in el) attrs.checked = String(el.checked);
    if (tag === 'option' && 'selected' in el) attrs.selected = String(el.selected);
    const compoundComponents = compoundComponentsFor(el);
    if (compoundComponents) attrs.compound_components = compoundComponents;
    const scroll = scrollInfoText(el);
    if (scroll) attrs.scroll = scroll;
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
  });
  const evalTreeForNode = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      return {
        node_type: 'text',
        node_value: node.nodeValue || ''
      };
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return null;
    return evalTreeForElement(node);
  };
  const evalTreeForElement = (el) => {
    if (isBrowserUseExcluded(el) || isNonContentTag(el)) return null;
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const attrs = collectAttributes(el, evalAttributeNames);
    const controlText = controlValueText(el);
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (controlText && type !== 'password') attrs.value = controlText;
    const axRef = el.getAttribute(axRefAttribute) || null;
    const children = Array.from(el.childNodes || []).map(evalTreeForNode).filter(Boolean);
    if (el.shadowRoot) {
      children.push({
        node_type: 'document_fragment',
        children: Array.from(el.shadowRoot.childNodes || []).map(evalTreeForNode).filter(Boolean)
      });
    }
    if (tag === 'iframe' || tag === 'frame') {
      try {
        const frameDocument = el.contentDocument;
        const frameRoot = frameDocument?.body || frameDocument?.documentElement;
        if (frameRoot) children.push(...Array.from(frameRoot.childNodes || []).map(evalTreeForNode).filter(Boolean));
      } catch (_) {}
    }
    const scroll = scrollInfoText(el);
    return {
      node_type: 'element',
      tag_name: tag,
      attributes: attrs,
      children,
      is_visible: isVisible(el),
      is_interactive: Boolean(axRef),
      is_scrollable: isScrollable(el),
      scroll_info: scroll || null,
      ax_ref: axRef
    };
  };
  return {
    stats,
    elements: indexedElements,
    eval_tree: evalTreeForElement(document.documentElement)
  };
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

const FRAME_ELEMENTS_JS: &str = r#"
JSON.stringify(Array.from(document.querySelectorAll('iframe,frame')).map((el) => {
  const rect = el.getBoundingClientRect();
  return {
    url: el.src || el.getAttribute('src') || '',
    x: Math.round(rect.x),
    y: Math.round(rect.y)
  };
}))
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

fn interactive_elements_js(config: IframeTraversalConfig, paint_order_filtering: bool) -> String {
    INTERACTIVE_ELEMENTS_JS
        .replace(
            "const maxIframeDepth = 5;",
            &format!(
                "const maxIframeDepth = {};",
                config.max_iframe_depth_for_same_origin()
            ),
        )
        .replace(
            "const maxIframeDocuments = 100;",
            &format!("const maxIframeDocuments = {};", config.max_iframes),
        )
        .replace(
            "const paintOrderFiltering = true;",
            &format!("const paintOrderFiltering = {paint_order_filtering};"),
        )
}

fn element_eval_js(index: u32, body: &str) -> String {
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

fn element_action_js(index: u32, action: &str) -> String {
    element_eval_js(index, &format!("{action}\n  return true;"))
}

fn element_function_js(body: &str) -> String {
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

#[derive(Debug, Clone, Error)]
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
    #[error("Browser Use Cloud authentication failed: {0}")]
    CloudAuth(String),
    #[error("Browser Use Cloud request failed: {0}")]
    Cloud(String),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub bypass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CloudProxyCountryCode {
    #[default]
    Unset,
    Disabled,
    Country(String),
}

impl CloudProxyCountryCode {
    #[must_use]
    pub fn disabled() -> Self {
        Self::Disabled
    }

    #[must_use]
    pub fn country(country_code: impl Into<String>) -> Self {
        Self::Country(country_code.into())
    }

    fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
}

impl JsonSchema for CloudProxyCountryCode {
    fn schema_name() -> String {
        "CloudProxyCountryCode".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "string" },
                { "type": "null" }
            ]
        }))
        .expect("valid CloudProxyCountryCode JSON schema")
    }
}

fn serialize_cloud_proxy_country_code<S>(
    value: &CloudProxyCountryCode,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        CloudProxyCountryCode::Unset => serializer.serialize_none(),
        CloudProxyCountryCode::Disabled => serializer.serialize_none(),
        CloudProxyCountryCode::Country(country_code) => serializer.serialize_str(country_code),
    }
}

fn deserialize_cloud_proxy_country_code<'de, D>(
    deserializer: D,
) -> Result<CloudProxyCountryCode, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(country_code) => CloudProxyCountryCode::Country(country_code),
        None => CloudProxyCountryCode::Disabled,
    })
}

fn deserialize_env_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(values) = Option::<BTreeMap<String, Value>>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    values
        .into_iter()
        .map(|(key, value)| env_value_to_string(value).map(|value| (key, value)))
        .collect()
}

fn env_value_to_string<E>(value: Value) -> Result<String, E>
where
    E: serde::de::Error,
{
    match value {
        Value::String(value) => Ok(value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        other => Err(E::custom(format!(
            "browser env values must be strings, numbers, or booleans; got {other}"
        ))),
    }
}

fn deserialize_non_negative_f64_option<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<f64>::deserialize(deserializer)?;
    match value {
        Some(value) if value.is_finite() && value >= 0.0 => Ok(Some(value)),
        Some(value) => Err(serde::de::Error::custom(format!(
            "device_scale_factor must be a finite non-negative number; got {value}"
        ))),
        None => Ok(None),
    }
}

fn deserialize_non_negative_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "page-load wait seconds must be a finite non-negative number; got {value}"
        )))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudBrowserCreateRequest {
    #[serde(
        default,
        alias = "cloud_profile_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub profile_id: Option<String>,
    #[serde(
        default,
        alias = "cloud_proxy_country_code",
        skip_serializing_if = "CloudProxyCountryCode::is_unset",
        serialize_with = "serialize_cloud_proxy_country_code",
        deserialize_with = "deserialize_cloud_proxy_country_code"
    )]
    pub proxy_country_code: CloudProxyCountryCode,
    #[serde(
        default,
        alias = "cloud_timeout",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<u32>,
    #[serde(default, alias = "enableRecording", skip_serializing_if = "is_false")]
    pub enable_recording: bool,
}

pub type CreateCloudBrowserRequest = CloudBrowserCreateRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudBrowserResponse {
    pub id: String,
    pub status: String,
    #[serde(rename = "liveUrl", alias = "live_url")]
    pub live_url: String,
    #[serde(rename = "cdpUrl", alias = "cdp_url")]
    pub cdp_url: String,
    #[serde(rename = "timeoutAt", alias = "timeout_at")]
    pub timeout_at: String,
    #[serde(rename = "startedAt", alias = "started_at")]
    pub started_at: String,
    #[serde(
        default,
        rename = "finishedAt",
        alias = "finished_at",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<String>,
}

impl CloudBrowserResponse {
    pub fn devtools_endpoint(&self) -> Result<DevToolsEndpoint, BrowserError> {
        DevToolsEndpoint::from_cdp_url(&self.cdp_url)
    }
}

pub struct CloudBrowserClient {
    api_base_url: String,
    api_key: Option<String>,
    auth_config_path: Option<PathBuf>,
    client: reqwest::Client,
    current_session_id: Arc<Mutex<Option<String>>>,
}

impl Default for CloudBrowserClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudBrowserClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            api_base_url: "https://api.browser-use.com".to_owned(),
            api_key: None,
            auth_config_path: None,
            client: cloud_http_client(),
            current_session_id: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            ..Self::new()
        }
    }

    #[must_use]
    pub fn with_api_base_url(mut self, api_base_url: impl Into<String>) -> Self {
        self.api_base_url = api_base_url.into().trim_end_matches('/').to_owned();
        self
    }

    #[must_use]
    pub fn with_base_url(self, api_base_url: impl Into<String>) -> Self {
        self.with_api_base_url(api_base_url)
    }

    #[must_use]
    pub fn with_auth_config_path(mut self, auth_config_path: impl Into<PathBuf>) -> Self {
        self.auth_config_path = Some(auth_config_path.into());
        self
    }

    pub async fn current_session_id(&self) -> Option<String> {
        self.current_session_id.lock().await.clone()
    }

    pub async fn create_browser(
        &self,
        request: &CloudBrowserCreateRequest,
    ) -> Result<CloudBrowserResponse, BrowserError> {
        self.create_browser_with_headers(request, std::iter::empty::<(&str, &str)>())
            .await
    }

    pub async fn create_browser_with_headers<K, V, I>(
        &self,
        request: &CloudBrowserCreateRequest,
        extra_headers: I,
    ) -> Result<CloudBrowserResponse, BrowserError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        let api_key = self.api_key()?;
        let url = format!("{}/api/v2/browsers", self.api_base_url);
        let headers = cloud_request_headers(api_key, extra_headers)?;
        let body =
            serde_json::to_vec(request).map_err(|error| BrowserError::Cloud(error.to_string()))?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| cloud_request_error("creating", error))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(BrowserError::CloudAuth(
                "BROWSER_USE_API_KEY is invalid. Get a new key at https://cloud.browser-use.com/new-api-key?utm_source=oss&utm_medium=use_cloud"
                    .to_owned(),
            ));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(BrowserError::CloudAuth(
                "Access forbidden. Please check your Browser Use Cloud subscription status."
                    .to_owned(),
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(BrowserError::Cloud(format!(
                "Failed to create cloud browser: HTTP {status}{}",
                render_cloud_error_body(&body)
            )));
        }
        let response = response
            .json::<CloudBrowserResponse>()
            .await
            .map_err(|error| {
                BrowserError::Cloud(format!("Unexpected error creating cloud browser: {error}"))
            })?;
        *self.current_session_id.lock().await = Some(response.id.clone());
        Ok(response)
    }

    pub async fn stop_browser(
        &self,
        session_id: Option<&str>,
    ) -> Result<CloudBrowserResponse, BrowserError> {
        self.stop_browser_with_headers(session_id, std::iter::empty::<(&str, &str)>())
            .await
    }

    pub async fn stop_browser_with_headers<K, V, I>(
        &self,
        session_id: Option<&str>,
        extra_headers: I,
    ) -> Result<CloudBrowserResponse, BrowserError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        let session_id = match session_id {
            Some(session_id) if !session_id.trim().is_empty() => session_id.to_owned(),
            _ => self.current_session_id().await.ok_or_else(|| {
                BrowserError::Cloud(
                    "No session ID provided and no current session available".to_owned(),
                )
            })?,
        };
        let api_key = self.api_key()?;
        let url = format!("{}/api/v2/browsers/{session_id}", self.api_base_url);
        let headers = cloud_request_headers(api_key, extra_headers)?;
        let body = serde_json::to_vec(&serde_json::json!({ "action": "stop" }))
            .map_err(|error| BrowserError::Cloud(error.to_string()))?;
        let response = self
            .client
            .patch(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| cloud_request_error("stopping", error))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(BrowserError::CloudAuth(
                "Authentication failed. Please make sure BROWSER_USE_API_KEY is set for Browser Use Cloud."
                    .to_owned(),
            ));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            self.clear_current_session_if(&session_id).await;
            return Err(BrowserError::Cloud(format!(
                "Cloud browser session {session_id} not found"
            )));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(BrowserError::Cloud(format!(
                "Failed to stop cloud browser: HTTP {status}{}",
                render_cloud_error_body(&body)
            )));
        }
        let response = response
            .json::<CloudBrowserResponse>()
            .await
            .map_err(|error| {
                BrowserError::Cloud(format!("Unexpected error stopping cloud browser: {error}"))
            })?;
        self.clear_current_session_if(&session_id).await;
        Ok(response)
    }

    pub async fn close(&self) {
        let _ = self.stop_browser(None).await;
    }

    fn api_key(&self) -> Result<String, BrowserError> {
        resolve_cloud_api_key(
            self.api_key.as_deref(),
            std::env::var("BROWSER_USE_API_KEY").ok(),
            self.auth_config_path.as_deref(),
        )
        .ok_or_else(|| {
                BrowserError::CloudAuth(
                    "BROWSER_USE_API_KEY is not set. To use cloud browsers, get a key at https://cloud.browser-use.com/new-api-key?utm_source=oss&utm_medium=use_cloud"
                        .to_owned(),
                )
            })
    }

    async fn clear_current_session_if(&self, session_id: &str) {
        let mut current_session_id = self.current_session_id.lock().await;
        if current_session_id.as_deref() == Some(session_id) {
            *current_session_id = None;
        }
    }
}

fn cloud_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CLOUD_HTTP_TIMEOUT)
        .build()
        .expect("valid Browser Use Cloud HTTP client")
}

fn download_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CLOUD_HTTP_TIMEOUT)
        .build()
        .expect("valid Browser Use download HTTP client")
}

fn cloud_request_headers<K, V, I>(
    api_key: String,
    extra_headers: I,
) -> Result<reqwest::header::HeaderMap, BrowserError>
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: IntoIterator<Item = (K, V)>,
{
    let mut headers = reqwest::header::HeaderMap::new();
    let api_key = reqwest::header::HeaderValue::from_str(&api_key)
        .map_err(|error| BrowserError::Cloud(format!("Invalid cloud API key header: {error}")))?;
    headers.insert("X-Browser-Use-API-Key", api_key);
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in extra_headers {
        let name = name.as_ref();
        let header_name =
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                BrowserError::Cloud(format!("Invalid cloud extra header name {name:?}: {error}"))
            })?;
        let value = value.as_ref();
        let header_value = reqwest::header::HeaderValue::from_str(value).map_err(|error| {
            BrowserError::Cloud(format!(
                "Invalid cloud extra header value for {header_name}: {error}"
            ))
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn cloud_request_error(action: &str, error: reqwest::Error) -> BrowserError {
    if error.is_timeout() {
        return BrowserError::Cloud(format!(
            "Timeout while {action} cloud browser. Please try again."
        ));
    }
    if error.is_connect() {
        return BrowserError::Cloud(
            "Failed to connect to cloud browser service. Please check your internet connection."
                .to_owned(),
        );
    }
    BrowserError::Cloud(format!("Unexpected error {action} cloud browser: {error}"))
}

fn resolve_cloud_api_key(
    explicit_api_key: Option<&str>,
    env_api_key: Option<String>,
    auth_config_path: Option<&Path>,
) -> Option<String> {
    explicit_api_key
        .and_then(non_empty_string)
        .or_else(|| env_api_key.as_deref().and_then(non_empty_string))
        .or_else(|| load_cloud_auth_api_token(auth_config_path))
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn load_cloud_auth_api_token(auth_config_path: Option<&Path>) -> Option<String> {
    let path = auth_config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_cloud_auth_config_path);
    let contents = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&contents).ok()?;
    value
        .get("api_token")
        .and_then(Value::as_str)
        .and_then(|token| {
            let token = token.trim();
            (!token.is_empty()).then(|| token.to_owned())
        })
}

fn default_cloud_auth_config_path() -> PathBuf {
    cloud_auth_config_path(
        std::env::var_os("BROWSER_USE_CONFIG_DIR").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn cloud_auth_config_path(
    browser_use_config_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    let config_dir = browser_use_config_dir
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| expand_home(path, home.as_deref()))
        .unwrap_or_else(|| {
            xdg_config_home
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| expand_home(path, home.as_deref()))
                .unwrap_or_else(|| expand_home(PathBuf::from("~/.config"), home.as_deref()))
                .join("browseruse")
        });
    config_dir.join("cloud_auth.json")
}

fn expand_home(path: PathBuf, home: Option<&Path>) -> PathBuf {
    let Some(path_text) = path.to_str() else {
        return path;
    };
    if path_text == "~" {
        return home.map(Path::to_path_buf).unwrap_or(path);
    }
    if let Some(rest) = path_text.strip_prefix("~/") {
        return home
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(path_text));
    }
    path
}

fn render_cloud_error_body(body: &str) -> String {
    if body.trim().is_empty() {
        return String::new();
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("detail")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| Some(body.to_owned()))
        .map(|detail| format!(" - {detail}"))
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum IgnoreDefaultArgs {
    All(bool),
    List(Vec<String>),
}

impl Default for IgnoreDefaultArgs {
    fn default() -> Self {
        Self::List(default_ignore_default_args())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BrowserChannel {
    #[serde(rename = "chromium")]
    Chromium,
    #[serde(rename = "chrome")]
    Chrome,
    #[serde(rename = "chrome-beta")]
    ChromeBeta,
    #[serde(rename = "chrome-dev")]
    ChromeDev,
    #[serde(rename = "chrome-canary")]
    ChromeCanary,
    #[serde(rename = "msedge")]
    MsEdge,
    #[serde(rename = "msedge-beta")]
    MsEdgeBeta,
    #[serde(rename = "msedge-dev")]
    MsEdgeDev,
    #[serde(rename = "msedge-canary")]
    MsEdgeCanary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_cloud: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_browser_params: Option<CloudBrowserCreateRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_key: Option<String>,
    #[serde(default, deserialize_with = "deserialize_env_map")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<BrowserChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_debugging_port: Option<u16>,
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default)]
    pub devtools: bool,
    #[serde(default = "default_chromium_sandbox")]
    pub chromium_sandbox: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    #[serde(default = "default_profile_directory")]
    pub profile_directory: String,
    #[serde(
        default,
        alias = "downloads_dir",
        alias = "save_downloads_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub downloads_path: Option<PathBuf>,
    #[serde(default = "default_accept_downloads")]
    pub accept_downloads: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_state_path: Option<PathBuf>,
    #[serde(default = "default_auto_download_pdfs")]
    pub auto_download_pdfs: bool,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub ignore_default_args: IgnoreDefaultArgs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default = "default_browser_permissions")]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    #[serde(default)]
    pub block_ip_addresses: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<bool>,
    #[serde(default)]
    pub disable_security: bool,
    #[serde(default)]
    pub deterministic_rendering: bool,
    #[serde(default = "default_cross_origin_iframes")]
    pub cross_origin_iframes: bool,
    #[serde(default = "default_max_iframes")]
    pub max_iframes: usize,
    #[serde(default = "default_max_iframe_depth")]
    pub max_iframe_depth: usize,
    #[serde(default = "default_paint_order_filtering")]
    pub paint_order_filtering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<BrowserViewport>,
    #[serde(default)]
    pub viewport: BrowserViewport,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_viewport: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_negative_f64_option"
    )]
    pub device_scale_factor: Option<f64>,
    #[serde(
        default = "default_minimum_wait_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub minimum_wait_page_load_time: f64,
    #[serde(
        default = "default_wait_for_network_idle_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub wait_for_network_idle_page_load_time: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<BrowserViewport>,
    #[serde(
        default = "default_window_position",
        skip_serializing_if = "Option::is_none"
    )]
    pub window_position: Option<BrowserViewport>,
    #[serde(default = "default_browser_start_timeout_ms")]
    pub browser_start_timeout_ms: u64,
    #[serde(default = "default_navigation_timeout_ms")]
    pub navigation_timeout_ms: u64,
    #[serde(default = "default_network_request_timeout_ms")]
    pub network_request_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
}

impl Default for BrowserProfile {
    fn default() -> Self {
        Self {
            cdp_url: None,
            headers: None,
            use_cloud: false,
            cloud_browser_params: None,
            cloud_api_base_url: None,
            cloud_api_key: None,
            env: BTreeMap::new(),
            executable_path: None,
            channel: None,
            remote_debugging_port: None,
            headless: default_headless(),
            devtools: false,
            chromium_sandbox: default_chromium_sandbox(),
            user_data_dir: None,
            profile_directory: default_profile_directory(),
            downloads_path: None,
            accept_downloads: default_accept_downloads(),
            storage_state_path: None,
            auto_download_pdfs: default_auto_download_pdfs(),
            args: Vec::new(),
            ignore_default_args: IgnoreDefaultArgs::default(),
            user_agent: None,
            permissions: default_browser_permissions(),
            allowed_domains: Vec::new(),
            prohibited_domains: Vec::new(),
            block_ip_addresses: false,
            keep_alive: None,
            disable_security: false,
            deterministic_rendering: false,
            cross_origin_iframes: default_cross_origin_iframes(),
            max_iframes: default_max_iframes(),
            max_iframe_depth: default_max_iframe_depth(),
            paint_order_filtering: default_paint_order_filtering(),
            screen: None,
            viewport: BrowserViewport::default(),
            no_viewport: false,
            device_scale_factor: None,
            minimum_wait_page_load_time: default_minimum_wait_page_load_time(),
            wait_for_network_idle_page_load_time: default_wait_for_network_idle_page_load_time(),
            window_size: None,
            window_position: default_window_position(),
            browser_start_timeout_ms: default_browser_start_timeout_ms(),
            navigation_timeout_ms: default_navigation_timeout_ms(),
            network_request_timeout_ms: default_network_request_timeout_ms(),
            proxy: None,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_headless() -> bool {
    true
}

fn default_chromium_sandbox() -> bool {
    true
}

fn default_profile_directory() -> String {
    "Default".to_owned()
}

fn default_browser_permissions() -> Vec<String> {
    ["clipboardReadWrite", "notifications"]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
}

fn default_auto_download_pdfs() -> bool {
    true
}

fn default_accept_downloads() -> bool {
    true
}

fn default_window_position() -> Option<BrowserViewport> {
    Some(BrowserViewport {
        width: 0,
        height: 0,
    })
}

fn default_cross_origin_iframes() -> bool {
    true
}

fn default_max_iframes() -> usize {
    100
}

fn default_max_iframe_depth() -> usize {
    5
}

fn default_paint_order_filtering() -> bool {
    true
}

fn default_ignore_default_args() -> Vec<String> {
    DEFAULT_IGNORE_DEFAULT_ARGS
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect()
}

fn default_browser_start_timeout_ms() -> u64 {
    30_000
}

fn default_navigation_timeout_ms() -> u64 {
    20_000
}

fn default_network_request_timeout_ms() -> u64 {
    10_000
}

fn default_minimum_wait_page_load_time() -> f64 {
    0.25
}

fn default_wait_for_network_idle_page_load_time() -> f64 {
    0.5
}

fn profile_keeps_launched_browser_alive(profile: &BrowserProfile) -> bool {
    profile.keep_alive == Some(true)
}

const CHROME_DEFAULT_ARGS: &[&str] = &[
    "--disable-field-trial-config",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-back-forward-cache",
    "--disable-breakpad",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--no-default-browser-check",
    "--disable-dev-shm-usage",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--metrics-recording-only",
    "--no-first-run",
    "--no-service-autorun",
    "--export-tagged-pdf",
    "--disable-search-engine-choice-screen",
    "--unsafely-disable-devtools-self-xss-warnings",
    "--enable-features=NetworkService,NetworkServiceInProcess",
    "--enable-network-information-downlink-max",
    "--disable-sync",
];

const DEFAULT_IGNORE_DEFAULT_ARGS: &[&str] = &[
    "--enable-automation",
    "--disable-extensions",
    "--hide-scrollbars",
    "--disable-features=AcceptCHFrame,AutoExpandDetailsElement,AvoidUnnecessaryBeforeUnloadCheckSync,CertificateTransparencyComponentUpdater,DeferRendererTasksAfterInput,DestroyProfileOnBrowserClose,DialMediaRouteProvider,ExtensionManifestV2Disabled,GlobalMediaControls,HttpsUpgrades,ImprovedCookieControls,LazyFrameLoading,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate",
];

const CHROME_DISABLE_SECURITY_ARGS: &[&str] = &[
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-features=IsolateOrigins,site-per-process",
    "--allow-running-insecure-content",
    "--ignore-certificate-errors",
    "--ignore-ssl-errors",
    "--ignore-certificate-errors-spki-list",
];

const CHROME_DOCKER_ARGS: &[&str] = &[
    "--no-sandbox",
    "--disable-gpu-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage",
    "--no-xshm",
    "--no-zygote",
    "--disable-site-isolation-trials",
];

const CHROME_DETERMINISTIC_RENDERING_ARGS: &[&str] = &[
    "--deterministic-mode",
    "--js-flags=--random-seed=1157259159",
    "--force-device-scale-factor=2",
    "--enable-webgl",
    "--font-render-hinting=none",
    "--force-color-profile=srgb",
];

impl BrowserProfile {
    #[must_use]
    pub fn uses_cloud(&self) -> bool {
        self.use_cloud || self.cloud_browser_params.is_some()
    }

    #[must_use]
    pub fn cloud_create_request(&self) -> Option<CloudBrowserCreateRequest> {
        self.uses_cloud()
            .then(|| self.cloud_browser_params.clone().unwrap_or_default())
    }

    #[must_use]
    pub fn cloud_browser_request(&self) -> CloudBrowserCreateRequest {
        self.cloud_browser_params.clone().unwrap_or_default()
    }

    pub async fn create_cloud_endpoint(&self) -> Result<Option<DevToolsEndpoint>, BrowserError> {
        let client = self.cloud_browser_client();
        self.create_cloud_endpoint_with_client(&client).await
    }

    pub async fn create_cloud_endpoint_with_client(
        &self,
        client: &CloudBrowserClient,
    ) -> Result<Option<DevToolsEndpoint>, BrowserError> {
        let Some(request) = self.cloud_create_request() else {
            return Ok(None);
        };
        client
            .create_browser(&request)
            .await?
            .devtools_endpoint()
            .map(Some)
    }

    pub async fn create_cloud_devtools_endpoint(&self) -> Result<DevToolsEndpoint, BrowserError> {
        self.create_cloud_endpoint()
            .await?
            .ok_or_else(|| BrowserError::Cloud("cloud browser is not enabled".to_owned()))
    }

    fn cloud_browser_client(&self) -> CloudBrowserClient {
        let mut client = match &self.cloud_api_key {
            Some(api_key) => CloudBrowserClient::with_api_key(api_key.clone()),
            None => CloudBrowserClient::new(),
        };
        if let Some(api_base_url) = &self.cloud_api_base_url {
            client = client.with_base_url(api_base_url.clone());
        }
        client
    }

    pub fn resolve_executable(&self) -> Result<PathBuf, BrowserError> {
        resolve_chrome_executable(
            self.executable_path.as_deref(),
            std::env::var_os("BROWSER_USE_CHROME").map(PathBuf::from),
            browser_executable_candidates(self.channel),
        )
    }

    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        self.try_launch_plan()
            .expect("invalid BrowserProfile launch plan")
    }

    pub fn try_launch_plan(&self) -> Result<BrowserLaunchPlan, BrowserError> {
        if self.headless && self.devtools {
            return Err(BrowserError::LaunchFailed(
                "headless=True and devtools=True cannot both be set at the same time".to_owned(),
            ));
        }
        if self.headless && self.no_viewport {
            return Err(BrowserError::LaunchFailed(
                "headless=True and no_viewport=True cannot both be set at the same time".to_owned(),
            ));
        }
        Ok(self.build_launch_plan())
    }

    fn build_launch_plan(&self) -> BrowserLaunchPlan {
        let remote_debugging_port = self.remote_debugging_port.unwrap_or(0);
        let window_size = self
            .window_size
            .as_ref()
            .or(self.screen.as_ref())
            .unwrap_or(&self.viewport);
        let mut args = self.default_chrome_args();
        args.push(format!("--remote-debugging-port={remote_debugging_port}"));
        args.push(format!(
            "--window-size={},{}",
            window_size.width, window_size.height
        ));

        if let Some(window_position) = &self.window_position {
            args.push(format!(
                "--window-position={},{}",
                window_position.width, window_position.height
            ));
        }

        if self.headless {
            args.push("--headless=new".to_owned());
        }

        if self.devtools {
            args.push("--auto-open-devtools-for-tabs".to_owned());
        }

        if let Some(user_data_dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", user_data_dir.display()));
            if !self.profile_directory.is_empty() {
                args.push(format!("--profile-directory={}", self.profile_directory));
            }
        }

        if !self.chromium_sandbox {
            args.extend(CHROME_DOCKER_ARGS.iter().map(|arg| (*arg).to_owned()));
        }

        if self.disable_security {
            args.extend(
                CHROME_DISABLE_SECURITY_ARGS
                    .iter()
                    .map(|arg| (*arg).to_owned()),
            );
        }

        if self.deterministic_rendering {
            args.extend(
                CHROME_DETERMINISTIC_RENDERING_ARGS
                    .iter()
                    .map(|arg| (*arg).to_owned()),
            );
        }

        if let Some(proxy) = &self.proxy {
            let proxy_server = proxy.server.as_str();
            if !proxy_server.is_empty() {
                args.push(format!("--proxy-server={proxy_server}"));
                if let Some(proxy_bypass) = proxy.bypass.as_deref() {
                    if !proxy_bypass.is_empty() {
                        args.push(format!("--proxy-bypass-list={proxy_bypass}"));
                    }
                }
            }
        }

        if let Some(user_agent) = self.user_agent.as_deref().filter(|value| !value.is_empty()) {
            args.push(format!("--user-agent={user_agent}"));
        }

        args.extend(self.args.iter().cloned());
        let args = normalize_launch_args(args);

        BrowserLaunchPlan {
            executable_path: self.executable_path.clone(),
            args,
            env: self.env.clone(),
        }
    }

    fn default_chrome_args(&self) -> Vec<String> {
        match &self.ignore_default_args {
            IgnoreDefaultArgs::All(true) => Vec::new(),
            IgnoreDefaultArgs::All(false) => CHROME_DEFAULT_ARGS
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect(),
            IgnoreDefaultArgs::List(ignored_args) => CHROME_DEFAULT_ARGS
                .iter()
                .filter(|arg| !ignored_args.iter().any(|ignored| ignored == **arg))
                .map(|arg| (*arg).to_owned())
                .collect(),
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
        let plan = launch_profile.try_launch_plan()?;

        let mut command = Command::new(&executable_path);
        command
            .args(&plan.args)
            .envs(&plan.env)
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

fn normalize_launch_args(args: Vec<String>) -> Vec<String> {
    dedupe_launch_args_by_switch(merge_disable_features_args(args))
}

fn merge_disable_features_args(args: Vec<String>) -> Vec<String> {
    let mut feature_values = Vec::new();
    let mut last_disable_features_index = None;

    for (index, arg) in args.iter().enumerate() {
        let Some(value) = disable_features_value(arg) else {
            continue;
        };
        last_disable_features_index = Some(index);
        feature_values.extend(value.split(',').map(str::to_owned));
    }

    let Some(last_disable_features_index) = last_disable_features_index else {
        return args;
    };
    let Some(merged_features) = merged_disable_features_value(&feature_values) else {
        return args
            .into_iter()
            .filter(|arg| disable_features_value(arg).is_none())
            .collect();
    };
    let merged_arg = format!("--disable-features={merged_features}");

    args.into_iter()
        .enumerate()
        .filter_map(|(index, arg)| {
            if disable_features_value(&arg).is_none() {
                return Some(arg);
            }
            (index == last_disable_features_index).then(|| merged_arg.clone())
        })
        .collect()
}

fn disable_features_value(arg: &str) -> Option<&str> {
    arg.strip_prefix("--disable-features=")
}

fn merged_disable_features_value(values: &[String]) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut unique_features = Vec::new();
    for value in values {
        let feature = value.trim();
        if feature.is_empty() || !seen.insert(feature.to_owned()) {
            continue;
        }
        unique_features.push(feature.to_owned());
    }
    (!unique_features.is_empty()).then(|| unique_features.join(","))
}

fn dedupe_launch_args_by_switch(args: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for arg in args.into_iter().rev() {
        if seen.insert(launch_arg_key(&arg).to_owned()) {
            deduped.push(arg);
        }
    }
    deduped.reverse();
    deduped
}

fn launch_arg_key(arg: &str) -> &str {
    arg.split_once('=').map_or(arg, |(key, _)| key)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycleEventKind {
    BrowserConnected,
    BrowserCloseRequested,
    BrowserStopped,
    BrowserReconnecting,
    BrowserReconnected,
    BrowserDiagnostic,
    TargetCreated,
    TargetClosed,
    TargetSwitched,
    TargetCrashed,
    NavigationStarted,
    NavigationCompleted,
    NavigationFailed,
    NetworkTimeout,
    NavigationBlocked,
    CurrentTargetReset,
    CurrentTargetResetFailed,
    PopupClosed,
    PopupCloseFailed,
    JavaScriptDialogHandled,
    DownloadStarted,
    DownloadProgress,
    FileDownloaded,
    StorageStateSaved,
    StorageStateLoaded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLifecycleEvent {
    pub kind: BrowserLifecycleEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    pub message: String,
}

impl BrowserLifecycleEvent {
    pub fn new(
        kind: BrowserLifecycleEventKind,
        target_id: Option<String>,
        url: Option<String>,
        reason: Option<String>,
        error: Option<String>,
        details: BTreeMap<String, String>,
        message: String,
    ) -> Self {
        Self {
            kind,
            target_id,
            url,
            reason,
            error,
            details,
            message,
        }
    }

    pub fn browser_connected(url: impl Into<String>) -> Self {
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserConnected,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Browser connected at {url}"),
        )
    }

    pub fn browser_close_requested() -> Self {
        Self::new(
            BrowserLifecycleEventKind::BrowserCloseRequested,
            None,
            None,
            None,
            None,
            BTreeMap::new(),
            "Browser close requested".to_owned(),
        )
    }

    pub fn browser_stopped(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserStopped,
            None,
            None,
            Some(reason.clone()),
            None,
            BTreeMap::new(),
            format!("Browser stopped ({reason})"),
        )
    }

    pub fn browser_reconnecting(url: impl Into<String>, attempt: u32, max_attempts: u32) -> Self {
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserReconnecting,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("attempt".to_owned(), attempt.to_string()),
                ("max_attempts".to_owned(), max_attempts.to_string()),
            ]),
            format!("Browser reconnecting to {url} (attempt {attempt}/{max_attempts})"),
        )
    }

    pub fn browser_reconnected(
        url: impl Into<String>,
        attempt: u32,
        downtime_seconds: impl Into<String>,
    ) -> Self {
        let url = url.into();
        let downtime_seconds = downtime_seconds.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserReconnected,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("attempt".to_owned(), attempt.to_string()),
                ("downtime_seconds".to_owned(), downtime_seconds.clone()),
            ]),
            format!("Browser reconnected to {url} on attempt {attempt} after {downtime_seconds}s"),
        )
    }

    pub fn browser_diagnostic(
        reason: impl Into<String>,
        details: BTreeMap<String, String>,
        error: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserDiagnostic,
            None,
            None,
            Some(reason),
            error,
            details,
            message.into(),
        )
    }

    pub fn permissions_grant_failed(permissions: &[String], error: impl Into<String>) -> Self {
        let error = error.into();
        Self::browser_diagnostic(
            "permissions_grant_failed",
            BTreeMap::from([
                ("permissions".to_owned(), permissions.join(",")),
                (
                    "permissions_count".to_owned(),
                    permissions.len().to_string(),
                ),
            ]),
            Some(error.clone()),
            format!("Browser permission grant failed: {error}"),
        )
    }

    pub fn target_created(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::TargetCreated,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Target {target_id} created for {url}"),
        )
    }

    pub fn target_closed(target_id: impl Into<String>) -> Self {
        let target_id = target_id.into();
        Self::new(
            BrowserLifecycleEventKind::TargetClosed,
            Some(target_id.clone()),
            None,
            None,
            None,
            BTreeMap::new(),
            format!("Target {target_id} closed"),
        )
    }

    pub fn target_switched(target_id: impl Into<String>) -> Self {
        let target_id = target_id.into();
        Self::new(
            BrowserLifecycleEventKind::TargetSwitched,
            Some(target_id.clone()),
            None,
            None,
            None,
            BTreeMap::new(),
            format!("Agent focus switched to target {target_id}"),
        )
    }

    pub fn target_crashed(target_id: impl Into<String>, error: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::TargetCrashed,
            Some(target_id.clone()),
            None,
            None,
            Some(error.clone()),
            BTreeMap::new(),
            format!("Target {target_id} crashed: {error}"),
        )
    }

    pub fn navigation_started(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationStarted,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Navigation started on target {target_id} to {url}"),
        )
    }

    pub fn navigation_completed(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationCompleted,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Navigation completed on target {target_id} to {url}"),
        )
    }

    pub fn navigation_failed(
        target_id: impl Into<String>,
        url: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationFailed,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            Some(error.clone()),
            BTreeMap::new(),
            format!("Navigation failed on target {target_id} to {url}: {error}"),
        )
    }

    pub fn network_timeout(
        target_id: impl Into<String>,
        url: impl Into<String>,
        timeout_seconds: impl Into<String>,
    ) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        let timeout_seconds = timeout_seconds.into();
        Self::new(
            BrowserLifecycleEventKind::NetworkTimeout,
            Some(target_id.clone()),
            Some(url.clone()),
            Some("network_timeout".to_owned()),
            Some(format!("timed out after {timeout_seconds}s")),
            BTreeMap::from([("timeout_seconds".to_owned(), timeout_seconds.clone())]),
            format!("Network timeout on target {target_id} for {url} after {timeout_seconds}s"),
        )
    }

    pub fn javascript_dialog_handled(
        url: impl Into<String>,
        dialog_type: impl Into<String>,
        message: impl Into<String>,
        accepted: bool,
    ) -> Self {
        let url = url.into();
        let dialog_type = dialog_type.into();
        let message = message.into();
        let action = if accepted { "accepted" } else { "dismissed" };
        Self::new(
            BrowserLifecycleEventKind::JavaScriptDialogHandled,
            None,
            Some(url.clone()),
            Some(dialog_type.clone()),
            None,
            BTreeMap::from([
                ("dialog_type".to_owned(), dialog_type.clone()),
                ("dialog_message".to_owned(), message.clone()),
                ("action".to_owned(), action.to_owned()),
            ]),
            format!("JavaScript {dialog_type} dialog on {url} was {action}: {message}"),
        )
    }

    pub fn download_started(
        guid: impl Into<String>,
        url: impl Into<String>,
        suggested_filename: impl Into<String>,
    ) -> Self {
        let guid = guid.into();
        let url = url.into();
        let suggested_filename = suggested_filename.into();
        Self::new(
            BrowserLifecycleEventKind::DownloadStarted,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("suggested_filename".to_owned(), suggested_filename.clone()),
            ]),
            format!("Download {guid} started from {url} as {suggested_filename}"),
        )
    }

    pub fn download_progress(
        guid: impl Into<String>,
        received_bytes: u64,
        total_bytes: Option<u64>,
        state: impl Into<String>,
    ) -> Self {
        let guid = guid.into();
        let state = state.into();
        let mut details = BTreeMap::from([
            ("guid".to_owned(), guid.clone()),
            ("received_bytes".to_owned(), received_bytes.to_string()),
            ("state".to_owned(), state.clone()),
        ]);
        if let Some(total_bytes) = total_bytes {
            details.insert("total_bytes".to_owned(), total_bytes.to_string());
        }
        Self::new(
            BrowserLifecycleEventKind::DownloadProgress,
            None,
            None,
            Some(state.clone()),
            None,
            details,
            format!("Download {guid} progress: {state} ({received_bytes} bytes received)"),
        )
    }

    pub fn file_downloaded(
        guid: impl Into<String>,
        path: impl Into<String>,
        file_name: impl Into<String>,
        file_size: u64,
    ) -> Self {
        let guid = guid.into();
        let path = path.into();
        let file_name = file_name.into();
        Self::new(
            BrowserLifecycleEventKind::FileDownloaded,
            None,
            None,
            None,
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("path".to_owned(), path.clone()),
                ("file_name".to_owned(), file_name.clone()),
                ("file_size".to_owned(), file_size.to_string()),
            ]),
            format!("Download {guid} completed at {path} ({file_name}, {file_size} bytes)"),
        )
    }

    pub fn pdf_auto_downloaded(
        url: impl Into<String>,
        path: impl Into<String>,
        file_name: impl Into<String>,
        file_size: u64,
    ) -> Self {
        let url = url.into();
        let path = path.into();
        let file_name = file_name.into();
        let guid = format!("auto-pdf:{url}");
        Self::new(
            BrowserLifecycleEventKind::FileDownloaded,
            None,
            Some(url.clone()),
            Some("pdf_auto_download".to_owned()),
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("path".to_owned(), path.clone()),
                ("file_name".to_owned(), file_name.clone()),
                ("file_size".to_owned(), file_size.to_string()),
                ("auto_download".to_owned(), "true".to_owned()),
            ]),
            format!("Auto-downloaded PDF {url} to {path} ({file_name}, {file_size} bytes)"),
        )
    }

    pub fn pdf_auto_download_failed(url: impl Into<String>, error: impl Into<String>) -> Self {
        let url = url.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserDiagnostic,
            None,
            Some(url.clone()),
            Some("pdf_auto_download_failed".to_owned()),
            Some(error.clone()),
            BTreeMap::from([("auto_download".to_owned(), "true".to_owned())]),
            format!("Failed to auto-download PDF {url}: {error}"),
        )
    }

    pub fn storage_state_saved(
        path: impl Into<String>,
        cookies_count: usize,
        origins_count: usize,
    ) -> Self {
        let path = path.into();
        Self::new(
            BrowserLifecycleEventKind::StorageStateSaved,
            None,
            None,
            Some("storage_state".to_owned()),
            None,
            BTreeMap::from([
                ("path".to_owned(), path.clone()),
                ("cookies_count".to_owned(), cookies_count.to_string()),
                ("origins_count".to_owned(), origins_count.to_string()),
            ]),
            format!(
                "Storage state saved to {path} ({cookies_count} cookies, {origins_count} origins)"
            ),
        )
    }

    pub fn storage_state_loaded(
        path: impl Into<String>,
        cookies_count: usize,
        origins_count: usize,
    ) -> Self {
        let path = path.into();
        Self::new(
            BrowserLifecycleEventKind::StorageStateLoaded,
            None,
            None,
            Some("storage_state".to_owned()),
            None,
            BTreeMap::from([
                ("path".to_owned(), path.clone()),
                ("cookies_count".to_owned(), cookies_count.to_string()),
                ("origins_count".to_owned(), origins_count.to_string()),
            ]),
            format!(
                "Storage state loaded from {path} ({cookies_count} cookies, {origins_count} origins)"
            ),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycleAdapterEventKind {
    BrowserStop,
    BrowserConnected,
    BrowserStopped,
    BrowserReconnecting,
    BrowserReconnected,
    TabCreated,
    TabClosed,
    AgentFocusChanged,
    TargetCrashed,
    NavigationStarted,
    NavigationComplete,
    BrowserError,
    JavaScriptDialogHandled,
    DownloadStarted,
    DownloadProgress,
    FileDownloaded,
    StorageState,
    BrowserDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLifecycleAdapterEvent {
    pub kind: BrowserLifecycleAdapterEventKind,
    pub source_kind: BrowserLifecycleEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    pub message: String,
}

impl BrowserLifecycleAdapterEvent {
    pub fn from_lifecycle_event(event: &BrowserLifecycleEvent) -> Self {
        let kind = match &event.kind {
            BrowserLifecycleEventKind::BrowserConnected => {
                BrowserLifecycleAdapterEventKind::BrowserConnected
            }
            BrowserLifecycleEventKind::BrowserCloseRequested => {
                BrowserLifecycleAdapterEventKind::BrowserStop
            }
            BrowserLifecycleEventKind::BrowserStopped => {
                BrowserLifecycleAdapterEventKind::BrowserStopped
            }
            BrowserLifecycleEventKind::BrowserReconnecting => {
                BrowserLifecycleAdapterEventKind::BrowserReconnecting
            }
            BrowserLifecycleEventKind::BrowserReconnected => {
                BrowserLifecycleAdapterEventKind::BrowserReconnected
            }
            BrowserLifecycleEventKind::BrowserDiagnostic => {
                BrowserLifecycleAdapterEventKind::BrowserDiagnostic
            }
            BrowserLifecycleEventKind::TargetCreated => {
                BrowserLifecycleAdapterEventKind::TabCreated
            }
            BrowserLifecycleEventKind::TargetClosed => BrowserLifecycleAdapterEventKind::TabClosed,
            BrowserLifecycleEventKind::TargetSwitched => {
                BrowserLifecycleAdapterEventKind::AgentFocusChanged
            }
            BrowserLifecycleEventKind::TargetCrashed => {
                BrowserLifecycleAdapterEventKind::TargetCrashed
            }
            BrowserLifecycleEventKind::NavigationStarted => {
                BrowserLifecycleAdapterEventKind::NavigationStarted
            }
            BrowserLifecycleEventKind::NavigationCompleted => {
                BrowserLifecycleAdapterEventKind::NavigationComplete
            }
            BrowserLifecycleEventKind::NavigationFailed
            | BrowserLifecycleEventKind::NetworkTimeout
            | BrowserLifecycleEventKind::NavigationBlocked
            | BrowserLifecycleEventKind::CurrentTargetResetFailed
            | BrowserLifecycleEventKind::PopupCloseFailed => {
                BrowserLifecycleAdapterEventKind::BrowserError
            }
            BrowserLifecycleEventKind::CurrentTargetReset
            | BrowserLifecycleEventKind::PopupClosed => {
                BrowserLifecycleAdapterEventKind::BrowserDiagnostic
            }
            BrowserLifecycleEventKind::JavaScriptDialogHandled => {
                BrowserLifecycleAdapterEventKind::JavaScriptDialogHandled
            }
            BrowserLifecycleEventKind::DownloadStarted => {
                BrowserLifecycleAdapterEventKind::DownloadStarted
            }
            BrowserLifecycleEventKind::DownloadProgress => {
                BrowserLifecycleAdapterEventKind::DownloadProgress
            }
            BrowserLifecycleEventKind::FileDownloaded => {
                BrowserLifecycleAdapterEventKind::FileDownloaded
            }
            BrowserLifecycleEventKind::StorageStateSaved
            | BrowserLifecycleEventKind::StorageStateLoaded => {
                BrowserLifecycleAdapterEventKind::StorageState
            }
        };

        Self {
            kind,
            source_kind: event.kind.clone(),
            target_id: event.target_id.clone(),
            url: event.url.clone(),
            reason: event.reason.clone(),
            error: event.error.clone(),
            details: event.details.clone(),
            message: event.message.clone(),
        }
    }
}

pub fn browser_lifecycle_adapter_events(
    events: &[BrowserLifecycleEvent],
) -> Vec<BrowserLifecycleAdapterEvent> {
    events
        .iter()
        .map(BrowserLifecycleAdapterEvent::from_lifecycle_event)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BrowserLifecycleEventStreamError {
    #[error("lifecycle event stream closed")]
    Closed,
    #[error("lifecycle event stream lagged by {0} events")]
    Lagged(u64),
}

#[derive(Debug)]
pub struct BrowserLifecycleEventSubscription {
    receiver: broadcast::Receiver<BrowserLifecycleEvent>,
}

impl BrowserLifecycleEventSubscription {
    fn new(receiver: broadcast::Receiver<BrowserLifecycleEvent>) -> Self {
        Self { receiver }
    }

    pub fn closed() -> Self {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        Self::new(receiver)
    }

    pub async fn recv(
        &mut self,
    ) -> Result<BrowserLifecycleEvent, BrowserLifecycleEventStreamError> {
        match self.receiver.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Closed) => {
                Err(BrowserLifecycleEventStreamError::Closed)
            }
            Err(broadcast::error::RecvError::Lagged(count)) => {
                Err(BrowserLifecycleEventStreamError::Lagged(count))
            }
        }
    }

    pub fn try_recv(
        &mut self,
    ) -> Result<Option<BrowserLifecycleEvent>, BrowserLifecycleEventStreamError> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => {
                Err(BrowserLifecycleEventStreamError::Closed)
            }
            Err(broadcast::error::TryRecvError::Lagged(count)) => {
                Err(BrowserLifecycleEventStreamError::Lagged(count))
            }
        }
    }

    pub fn resubscribe(&self) -> Self {
        Self::new(self.receiver.resubscribe())
    }
}

#[derive(Debug)]
pub struct BrowserLifecycleAdapterEventSubscription {
    subscription: BrowserLifecycleEventSubscription,
}

impl BrowserLifecycleAdapterEventSubscription {
    pub fn new(subscription: BrowserLifecycleEventSubscription) -> Self {
        Self { subscription }
    }

    pub fn closed() -> Self {
        Self::new(BrowserLifecycleEventSubscription::closed())
    }

    pub async fn recv(
        &mut self,
    ) -> Result<BrowserLifecycleAdapterEvent, BrowserLifecycleEventStreamError> {
        self.subscription
            .recv()
            .await
            .map(|event| BrowserLifecycleAdapterEvent::from_lifecycle_event(&event))
    }

    pub fn try_recv(
        &mut self,
    ) -> Result<Option<BrowserLifecycleAdapterEvent>, BrowserLifecycleEventStreamError> {
        self.subscription.try_recv().map(|event| {
            event.map(|event| BrowserLifecycleAdapterEvent::from_lifecycle_event(&event))
        })
    }

    pub fn resubscribe(&self) -> Self {
        Self::new(self.subscription.resubscribe())
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
pub fn browser_executable_candidates(channel: Option<BrowserChannel>) -> Vec<PathBuf> {
    match channel {
        Some(channel) => browser_channel_candidates(channel),
        None => default_chrome_candidates(),
    }
}

#[must_use]
pub fn browser_channel_candidates(channel: BrowserChannel) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        match channel {
            BrowserChannel::Chromium => candidates.push(PathBuf::from(
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            )),
            BrowserChannel::Chrome => candidates.push(PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            )),
            BrowserChannel::ChromeBeta => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            )),
            BrowserChannel::ChromeDev => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
            )),
            BrowserChannel::ChromeCanary => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            )),
            BrowserChannel::MsEdge => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            )),
            BrowserChannel::MsEdgeBeta => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
            )),
            BrowserChannel::MsEdgeDev => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
            )),
            BrowserChannel::MsEdgeCanary => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
            )),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match channel {
            BrowserChannel::Chromium => {
                candidates.push(PathBuf::from("/usr/bin/chromium"));
                candidates.push(PathBuf::from("/usr/bin/chromium-browser"));
            }
            BrowserChannel::Chrome => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome"));
                candidates.push(PathBuf::from("/usr/bin/google-chrome-stable"));
            }
            BrowserChannel::ChromeBeta => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-beta"))
            }
            BrowserChannel::ChromeDev => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-unstable"))
            }
            BrowserChannel::ChromeCanary => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-canary"))
            }
            BrowserChannel::MsEdge => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge"));
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-stable"));
            }
            BrowserChannel::MsEdgeBeta => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-beta"))
            }
            BrowserChannel::MsEdgeDev => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-dev"))
            }
            BrowserChannel::MsEdgeCanary => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-canary"))
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let program_files = std::env::var_os("PROGRAMFILES").map(PathBuf::from);
        let program_files_x86 = std::env::var_os("PROGRAMFILES(X86)").map(PathBuf::from);
        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        match channel {
            BrowserChannel::Chromium => {
                if let Some(local_app_data) = &local_app_data {
                    candidates.push(local_app_data.join("Chromium/Application/chrome.exe"));
                }
            }
            BrowserChannel::Chrome => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Google/Chrome/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates.push(program_files_x86.join("Google/Chrome/Application/chrome.exe"));
                }
                if let Some(local_app_data) = &local_app_data {
                    candidates.push(local_app_data.join("Google/Chrome/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeBeta => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Google/Chrome Beta/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Google/Chrome Beta/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeDev => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Google/Chrome Dev/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Google/Chrome Dev/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeCanary => {
                if let Some(local_app_data) = &local_app_data {
                    candidates
                        .push(local_app_data.join("Google/Chrome SxS/Application/chrome.exe"));
                }
            }
            BrowserChannel::MsEdge => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Microsoft/Edge/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeBeta => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Microsoft/Edge Beta/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge Beta/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeDev => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Microsoft/Edge Dev/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge Dev/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeCanary => {
                if let Some(local_app_data) = &local_app_data {
                    candidates
                        .push(local_app_data.join("Microsoft/Edge SxS/Application/msedge.exe"));
                }
            }
        }
    }

    candidates
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DevToolsEndpoint {
    pub http_url: String,
    pub websocket_url: String,
}

impl DevToolsEndpoint {
    pub fn from_cdp_url(cdp_url: &str) -> Result<Self, BrowserError> {
        let parsed = url::Url::parse(cdp_url)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let websocket_url = match parsed.scheme() {
            "ws" | "wss" => cdp_url.to_owned(),
            scheme => {
                return Err(BrowserError::StateUnavailable(format!(
                    "unsupported CDP URL scheme {scheme:?}; expected ws or wss"
                )));
            }
        };
        let mut http_url = parsed;
        let http_scheme = if http_url.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        http_url.set_scheme(http_scheme).map_err(|_| {
            BrowserError::StateUnavailable(format!(
                "could not convert CDP URL scheme to {http_scheme}"
            ))
        })?;
        http_url.set_path("");
        http_url.set_query(None);
        http_url.set_fragment(None);
        Ok(Self {
            http_url: http_url.to_string().trim_end_matches('/').to_owned(),
            websocket_url,
        })
    }

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
    request_tx: mpsc::Sender<CdpRequest>,
    event_tx: broadcast::Sender<CdpEvent>,
    next_id: AtomicU64,
    intentional_stop: Arc<AtomicBool>,
    connection_generation: Arc<AtomicU64>,
    session_generations: Arc<Mutex<HashMap<String, u64>>>,
}

struct CdpRequest {
    id: u64,
    method: String,
    payload: Value,
    response_tx: oneshot::Sender<Result<Value, BrowserError>>,
}

struct CdpSocketConfig {
    cdp_url: String,
    websocket_url: String,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
struct CdpEvent {
    method: String,
    params: Value,
    session_id: Option<String>,
}

impl CdpConnection {
    pub async fn connect(endpoint: &DevToolsEndpoint) -> Result<Arc<Self>, BrowserError> {
        Self::connect_with_headers(endpoint, None).await
    }

    async fn connect_with_headers(
        endpoint: &DevToolsEndpoint,
        headers: Option<&BTreeMap<String, String>>,
    ) -> Result<Arc<Self>, BrowserError> {
        let headers = headers.cloned().unwrap_or_default();
        let socket = connect_cdp_socket(&endpoint.websocket_url, &headers).await?;
        let (request_tx, request_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel(256);
        let intentional_stop = Arc::new(AtomicBool::new(false));
        let connection_generation = Arc::new(AtomicU64::new(0));
        let session_generations = Arc::new(Mutex::new(HashMap::new()));
        let socket_config = CdpSocketConfig {
            cdp_url: endpoint.http_url.clone(),
            websocket_url: endpoint.websocket_url.clone(),
            headers,
        };
        tokio::spawn(cdp_connection_actor(
            socket_config,
            socket,
            request_rx,
            event_tx.clone(),
            intentional_stop.clone(),
            connection_generation.clone(),
        ));

        Ok(Arc::new(Self {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop,
            connection_generation,
            session_generations,
        }))
    }

    fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    fn mark_intentional_stop(&self) {
        self.intentional_stop.store(true, Ordering::Relaxed);
    }

    fn current_generation(&self) -> u64 {
        self.connection_generation.load(Ordering::Relaxed)
    }

    async fn register_attached_session(&self, session_id: &str) {
        self.session_generations
            .lock()
            .await
            .insert(session_id.to_owned(), self.current_generation());
    }

    async fn ensure_session_generation_current(
        &self,
        session_id: Option<&str>,
    ) -> Result<(), BrowserError> {
        let Some(session_id) = session_id else {
            return Ok(());
        };
        if self.is_registered_session_stale(session_id).await {
            return Err(BrowserError::Transport(format!(
                "CDP session {session_id} is stale after reconnect; reattach target before sending session-scoped commands"
            )));
        }
        Ok(())
    }

    async fn is_registered_session_stale(&self, session_id: &str) -> bool {
        let Some(session_generation) = self
            .session_generations
            .lock()
            .await
            .get(session_id)
            .copied()
        else {
            return false;
        };
        session_generation != self.current_generation()
    }

    pub async fn command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, BrowserError> {
        self.ensure_session_generation_current(session_id).await?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        if let Some(session_id) = session_id {
            request["sessionId"] = Value::String(session_id.to_owned());
        }

        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(CdpRequest {
                id,
                method: method.to_owned(),
                payload: request,
                response_tx,
            })
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        response_rx
            .await
            .map_err(|_| BrowserError::Transport("CDP command actor stopped".to_owned()))?
    }
}

async fn connect_cdp_socket(
    websocket_url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<CdpSocket, BrowserError> {
    let request = cdp_websocket_request(websocket_url, headers)?;
    let connect_result = tokio::time::timeout(
        Duration::from_millis(CDP_CONNECT_TIMEOUT_MS),
        connect_async(request),
    )
    .await
    .map_err(|_| {
        BrowserError::Transport(format!(
            "CDP websocket connect to {websocket_url} timed out after {CDP_CONNECT_TIMEOUT_MS}ms"
        ))
    })?;
    connect_result
        .map(|(socket, _)| socket)
        .map_err(|error| BrowserError::Transport(error.to_string()))
}

fn cdp_websocket_request(
    websocket_url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, BrowserError> {
    let mut request = websocket_url
        .into_client_request()
        .map_err(|error| BrowserError::Transport(error.to_string()))?;
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            BrowserError::Transport(format!(
                "invalid CDP websocket header name {name:?}: {error}"
            ))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            BrowserError::Transport(format!(
                "invalid CDP websocket header value for {name:?}: {error}"
            ))
        })?;
        request.headers_mut().insert(header_name, header_value);
    }
    Ok(request)
}

async fn cdp_connection_actor(
    socket_config: CdpSocketConfig,
    mut socket: CdpSocket,
    mut request_rx: mpsc::Receiver<CdpRequest>,
    event_tx: broadcast::Sender<CdpEvent>,
    intentional_stop: Arc<AtomicBool>,
    connection_generation: Arc<AtomicU64>,
) {
    let mut pending: HashMap<u64, (String, oneshot::Sender<Result<Value, BrowserError>>)> =
        HashMap::new();

    loop {
        let websocket_closed_event = loop {
            tokio::select! {
                Some(request) = request_rx.recv() => {
                    let text = request.payload.to_string();
                    match socket.send(Message::Text(text.into())).await {
                        Ok(()) => {
                            pending.insert(request.id, (request.method, request.response_tx));
                        }
                        Err(error) => {
                            let _ = request.response_tx.send(Err(BrowserError::Transport(error.to_string())));
                        }
                    }
                }
                message = socket.next() => {
                    let Some(message) = message else {
                        break cdp_websocket_closed_event("websocket_stream_ended", None);
                    };
                    let payload = match message {
                        Ok(Message::Text(text)) => match serde_json::from_str::<Value>(&text) {
                            Ok(payload) => payload,
                            Err(error) => {
                                let _ = event_tx.send(CdpEvent {
                                    method: "browser-use-rs.invalid-json".to_owned(),
                                    params: json!({ "error": error.to_string() }),
                                    session_id: None,
                                });
                                continue;
                            }
                        },
                        Ok(_) => continue,
                        Err(error) => {
                            let error = error.to_string();
                            let transport_error = BrowserError::Transport(error.clone());
                            for (_, (_, response_tx)) in pending.drain() {
                                let _ = response_tx.send(Err(transport_error.clone()));
                            }
                            break cdp_websocket_closed_event("websocket_error", Some(error));
                        }
                    };

                    if let Some(id) = payload.get("id").and_then(Value::as_u64) {
                        let Some((method, response_tx)) = pending.remove(&id) else {
                            continue;
                        };
                        let result = if let Some(error) = payload.get("error") {
                            let message = error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown CDP error")
                                .to_owned();
                            Err(BrowserError::CommandFailed { method, message })
                        } else {
                            payload
                                .get("result")
                                .cloned()
                                .ok_or_else(|| BrowserError::MissingResponseData(format!("{method} result")))
                        };
                        let _ = response_tx.send(result);
                        continue;
                    }

                    if let Some(method) = payload.get("method").and_then(Value::as_str) {
                        let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                        let session_id = payload
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        let _ = event_tx.send(CdpEvent {
                            method: method.to_owned(),
                            params,
                            session_id,
                        });
                    }
                }
                else => {
                    break cdp_websocket_closed_event("connection_actor_stopped", None);
                }
            }
        };

        let _ = event_tx.send(websocket_closed_event.clone());

        for (_, (_, response_tx)) in pending.drain() {
            let _ = response_tx.send(Err(BrowserError::Transport(
                "CDP websocket closed while waiting for response".to_owned(),
            )));
        }

        if !should_reconnect_after_websocket_event(
            &websocket_closed_event,
            intentional_stop.load(Ordering::Relaxed),
            request_rx.is_closed(),
        ) {
            break;
        }

        match reconnect_cdp_socket(&socket_config, &event_tx, &connection_generation).await {
            Some(reconnected_socket) => {
                socket = reconnected_socket;
            }
            None => break,
        }
    }
}

async fn reconnect_cdp_socket(
    socket_config: &CdpSocketConfig,
    event_tx: &broadcast::Sender<CdpEvent>,
    connection_generation: &AtomicU64,
) -> Option<CdpSocket> {
    let started_at = Instant::now();
    let mut last_error = None;

    for attempt in 1..=CDP_RECONNECT_MAX_ATTEMPTS {
        let _ = event_tx.send(cdp_websocket_reconnecting_event(
            &socket_config.cdp_url,
            attempt,
            CDP_RECONNECT_MAX_ATTEMPTS,
        ));

        match connect_cdp_socket(&socket_config.websocket_url, &socket_config.headers).await {
            Ok(socket) => {
                let generation = connection_generation.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = event_tx.send(cdp_websocket_reconnected_event(
                    &socket_config.cdp_url,
                    attempt,
                    started_at.elapsed(),
                    generation,
                ));
                return Some(socket);
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt < CDP_RECONNECT_MAX_ATTEMPTS {
                    sleep(cdp_reconnect_delay_for_attempt(attempt)).await;
                }
            }
        }
    }

    let _ = event_tx.send(cdp_websocket_reconnect_failed_event(
        &socket_config.cdp_url,
        CDP_RECONNECT_MAX_ATTEMPTS,
        started_at.elapsed(),
        last_error,
    ));
    None
}

fn should_reconnect_after_websocket_event(
    event: &CdpEvent,
    intentional_stop: bool,
    request_channel_closed: bool,
) -> bool {
    if intentional_stop || request_channel_closed {
        return false;
    }
    event.method == "browser-use-rs.websocket-closed"
        && event
            .params
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| matches!(reason, "websocket_stream_ended" | "websocket_error"))
}

fn cdp_reconnect_delay_for_attempt(attempt: u32) -> Duration {
    let index = attempt.saturating_sub(1) as usize;
    Duration::from_millis(
        CDP_RECONNECT_DELAYS_MS
            .get(index)
            .copied()
            .unwrap_or_else(|| *CDP_RECONNECT_DELAYS_MS.last().expect("nonempty delays")),
    )
}

fn cdp_websocket_closed_event(reason: &str, error: Option<String>) -> CdpEvent {
    let mut params = json!({ "reason": reason });
    if let Some(error) = error {
        params["error"] = Value::String(error);
    }
    CdpEvent {
        method: "browser-use-rs.websocket-closed".to_owned(),
        params,
        session_id: None,
    }
}

fn cdp_websocket_reconnecting_event(cdp_url: &str, attempt: u32, max_attempts: u32) -> CdpEvent {
    CdpEvent {
        method: "browser-use-rs.websocket-reconnecting".to_owned(),
        params: json!({
            "cdp_url": cdp_url,
            "attempt": attempt,
            "max_attempts": max_attempts,
        }),
        session_id: None,
    }
}

fn cdp_websocket_reconnected_event(
    cdp_url: &str,
    attempt: u32,
    downtime: Duration,
    generation: u64,
) -> CdpEvent {
    CdpEvent {
        method: "browser-use-rs.websocket-reconnected".to_owned(),
        params: json!({
            "cdp_url": cdp_url,
            "attempt": attempt,
            "downtime_seconds": format!("{:.3}", downtime.as_secs_f64()),
            "connection_generation": generation,
        }),
        session_id: None,
    }
}

fn cdp_websocket_reconnect_failed_event(
    cdp_url: &str,
    max_attempts: u32,
    downtime: Duration,
    error: Option<String>,
) -> CdpEvent {
    let mut params = json!({
        "cdp_url": cdp_url,
        "max_attempts": max_attempts,
        "downtime_seconds": format!("{:.3}", downtime.as_secs_f64()),
    });
    if let Some(error) = error {
        params["error"] = Value::String(error);
    }
    CdpEvent {
        method: "browser-use-rs.websocket-reconnect-failed".to_owned(),
        params,
        session_id: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPage {
    pub target_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IframeTraversalConfig {
    cross_origin_iframes: bool,
    max_iframes: usize,
    max_iframe_depth: usize,
}

impl IframeTraversalConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            cross_origin_iframes: profile.cross_origin_iframes,
            max_iframes: profile.max_iframes,
            max_iframe_depth: profile.max_iframe_depth,
        }
    }

    fn max_iframe_depth_for_same_origin(self) -> usize {
        self.max_iframe_depth
    }

    fn remaining_same_origin_depth(self, current_depth: usize) -> usize {
        self.max_iframe_depth.saturating_sub(current_depth)
    }

    fn allows_cross_origin_depth(self, depth: usize) -> bool {
        self.cross_origin_iframes && depth <= self.max_iframe_depth && self.max_iframes > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewportEmulationConfig {
    viewport: Option<BrowserViewport>,
    device_scale_factor: f64,
}

impl ViewportEmulationConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        let viewport = (!profile.no_viewport).then_some(profile.viewport);
        Self {
            viewport,
            device_scale_factor: profile.device_scale_factor.unwrap_or(1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageLoadWaitConfig {
    minimum_wait: Duration,
    network_idle_wait: Duration,
}

impl PageLoadWaitConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            minimum_wait: Duration::from_secs_f64(profile.minimum_wait_page_load_time),
            network_idle_wait: Duration::from_secs_f64(
                profile.wait_for_network_idle_page_load_time,
            ),
        }
    }

    fn is_disabled(self) -> bool {
        self.minimum_wait.is_zero() && self.network_idle_wait.is_zero()
    }
}

#[derive(Debug)]
struct NetworkActivityState {
    active_request_ids: BTreeSet<String>,
    last_activity_at: Instant,
}

impl NetworkActivityState {
    fn new(now: Instant) -> Self {
        Self {
            active_request_ids: BTreeSet::new(),
            last_activity_at: now,
        }
    }

    fn observe_request_started(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.insert(request_id.to_owned());
        self.last_activity_at = now;
    }

    fn observe_request_finished(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.remove(request_id);
        self.last_activity_at = now;
    }

    fn idle_remaining(&self, now: Instant, idle_for: Duration) -> Option<Duration> {
        if !self.active_request_ids.is_empty() {
            return Some(idle_for);
        }
        let elapsed = now.saturating_duration_since(self.last_activity_at);
        if elapsed >= idle_for {
            None
        } else {
            Some(idle_for - elapsed)
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FrameOffset {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameElementInfo {
    url: String,
    offset: FrameOffset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IframeTargetInfo {
    target_id: String,
    offset: FrameOffset,
    depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachedFramePage {
    page: AttachedPage,
    offset: FrameOffset,
    depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedDomElementRef {
    element: DomElementRef,
    target_local_index: u32,
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
    let canonical_host = canonical_ip_host(host);
    canonical_host.parse::<std::net::IpAddr>().is_ok()
        || parse_non_standard_ipv4(&canonical_host).is_some()
}

fn canonical_ip_host(host: &str) -> String {
    percent_decode_str(host.trim_matches(['[', ']']))
        .decode_utf8_lossy()
        .nfkc()
        .collect::<String>()
        .replace(['\u{3002}', '\u{ff61}'], ".")
}

fn parse_non_standard_ipv4(host: &str) -> Option<u32> {
    if host.is_empty()
        || host.contains(':')
        || host.contains('/')
        || host.chars().any(char::is_whitespace)
    {
        return None;
    }
    let parts = host
        .split('.')
        .map(parse_non_standard_ipv4_part)
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [a] if *a <= u32::MAX as u64 => Some(*a as u32),
        [a, b] if *a <= 0xff && *b <= 0x00ff_ffff => Some(((*a as u32) << 24) | (*b as u32)),
        [a, b, c] if *a <= 0xff && *b <= 0xff && *c <= 0xffff => {
            Some(((*a as u32) << 24) | ((*b as u32) << 16) | (*c as u32))
        }
        [a, b, c, d] if *a <= 0xff && *b <= 0xff && *c <= 0xff && *d <= 0xff => {
            Some(((*a as u32) << 24) | ((*b as u32) << 16) | ((*c as u32) << 8) | (*d as u32))
        }
        _ => None,
    }
}

fn parse_non_standard_ipv4_part(part: &str) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (radix, digits) =
        if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            (16, hex)
        } else if part.len() > 1 && part.starts_with('0') {
            (8, &part[1..])
        } else {
            (10, part)
        };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_digit(radix)) {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
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
    connection: Arc<CdpConnection>,
    page: Arc<Mutex<AttachedPage>>,
    last_dom_state: Arc<Mutex<Option<SerializedDomState>>>,
    pending_url_policy_error: Arc<Mutex<Option<BrowserError>>>,
    security_events: Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
    lifecycle_events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: broadcast::Sender<BrowserLifecycleEvent>,
    url_policy: UrlAccessPolicy,
    iframe_traversal: IframeTraversalConfig,
    paint_order_filtering: bool,
    viewport_emulation: ViewportEmulationConfig,
    page_load_wait: PageLoadWaitConfig,
    network_activity: Arc<Mutex<NetworkActivityState>>,
    downloads_path: Option<PathBuf>,
    auto_download_pdfs: bool,
    auto_pdf_downloads: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    storage_state_path: Option<PathBuf>,
    navigation_timeout_ms: u64,
    _lifecycle_watchdog: BrowserLifecycleWatchdog,
    _security_watchdog: Option<BrowserSecurityWatchdog>,
    _launched_browser: Option<LaunchedBrowser>,
    _downloads_dir: Option<TempDir>,
}

struct SessionDownloads {
    path: Option<PathBuf>,
    temp_dir: Option<TempDir>,
}

impl SessionDownloads {
    fn from_profile(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        if !profile.accept_downloads {
            return Ok(Self {
                path: None,
                temp_dir: None,
            });
        }
        if let Some(downloads_path) = &profile.downloads_path {
            return Ok(Self {
                path: Some(downloads_path.clone()),
                temp_dir: None,
            });
        }
        let temp_dir = tempfile::Builder::new()
            .prefix("browser-use-downloads-")
            .tempdir()
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(Self {
            path: Some(temp_dir.path().to_path_buf()),
            temp_dir: Some(temp_dir),
        })
    }
}

impl CdpBrowserSession {
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        Self::connect_with_profile(endpoint, &BrowserProfile::default()).await
    }

    pub async fn connect_with_profile(
        endpoint: DevToolsEndpoint,
        profile: &BrowserProfile,
    ) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            cdp_auto_pdf_download,
        );
        let page_load_wait = PageLoadWaitConfig::from_profile(profile);

        Ok(Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy: UrlAccessPolicy::from_profile(profile),
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait,
            network_activity,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: None,
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: None,
            _launched_browser: None,
            _downloads_dir: downloads.temp_dir,
        })
    }

    pub async fn launch(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let url_policy = UrlAccessPolicy::from_profile(profile);
        let (endpoint, launched_browser) = if profile.uses_cloud() {
            (profile.create_cloud_devtools_endpoint().await?, None)
        } else {
            let launched_browser = profile.launch_local().await?;
            (launched_browser.endpoint().clone(), Some(launched_browser))
        };
        let launched_browser = launched_browser.and_then(|browser| {
            if profile_keeps_launched_browser_alive(profile) {
                let _ = browser.detach();
                None
            } else {
                Some(browser)
            }
        });
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let storage_state_loaded_event = if let Some(storage_state_path) =
            &profile.storage_state_path
        {
            let storage_state = load_browser_storage_state(&connection, storage_state_path).await?;
            apply_origin_storage_state(&connection, &page, &storage_state).await?;
            let (cookies_count, origins_count) = storage_state_counts(&storage_state);
            Some(BrowserLifecycleEvent::storage_state_loaded(
                storage_state_path.display().to_string(),
                cookies_count,
                origins_count,
            ))
        } else {
            None
        };
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
            if let Some(event) = storage_state_loaded_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            cdp_auto_pdf_download,
        );
        let security_watchdog = BrowserSecurityWatchdog::start(
            connection.clone(),
            page.clone(),
            last_dom_state.clone(),
            pending_url_policy_error.clone(),
            security_events.clone(),
            LifecycleEventSink {
                events: lifecycle_events.clone(),
                event_tx: lifecycle_event_tx.clone(),
            },
            url_policy.clone(),
        )
        .await?;

        Ok(Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy,
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait: PageLoadWaitConfig::from_profile(profile),
            network_activity,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: profile.storage_state_path.clone(),
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: security_watchdog,
            _launched_browser: launched_browser,
            _downloads_dir: downloads.temp_dir,
        })
    }

    pub async fn close_browser(&self) -> Result<(), BrowserError> {
        self.record_lifecycle_event(BrowserLifecycleEvent::browser_close_requested())
            .await;
        if let Some(path) = &self.storage_state_path {
            self.save_storage_state(path).await?;
        }
        self.connection.mark_intentional_stop();
        self.connection
            .command("Browser.close", json!({}), None)
            .await
            .map(|_| ())
    }

    pub async fn save_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let storage_state = browser_storage_state(&self.connection, Some(&page)).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        write_storage_state(path, &storage_state).await?;
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_saved(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    pub async fn load_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let storage_state = load_browser_storage_state(&self.connection, path).await?;
        let page = self.current_page().await;
        apply_origin_storage_state(&self.connection, &page, &storage_state).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_loaded(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    async fn current_page(&self) -> AttachedPage {
        let page = self.page.lock().await.clone();
        if self
            .connection
            .is_registered_session_stale(&page.session_id)
            .await
        {
            return self
                .reattach_current_page(page.clone())
                .await
                .unwrap_or(page);
        }
        page
    }

    async fn set_current_page(&self, page: AttachedPage) {
        *self.page.lock().await = page;
    }

    async fn apply_viewport_emulation(&self, page: &AttachedPage) -> Result<(), BrowserError> {
        apply_viewport_emulation_for_page(&self.connection, page, self.viewport_emulation).await
    }

    async fn wait_for_page_load_settle(&self) {
        if self.page_load_wait.is_disabled() {
            return;
        }
        if !self.page_load_wait.minimum_wait.is_zero() {
            sleep(self.page_load_wait.minimum_wait).await;
        }
        if !self.page_load_wait.network_idle_wait.is_zero() {
            self.wait_for_network_idle(self.page_load_wait.network_idle_wait)
                .await;
        }
    }

    async fn wait_for_network_idle(&self, idle_for: Duration) {
        let deadline = Instant::now() + idle_for;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            let remaining = {
                self.network_activity
                    .lock()
                    .await
                    .idle_remaining(now, idle_for)
            };
            let Some(remaining) = remaining else {
                return;
            };
            let until_deadline = deadline.saturating_duration_since(now);
            let sleep_for = remaining.min(until_deadline).min(Duration::from_millis(50));
            if sleep_for.is_zero() {
                return;
            }
            sleep(sleep_for).await;
        }
    }

    async fn auto_download_pdf_if_needed(&self, url: &str) {
        if !self.auto_download_pdfs || !is_pdf_viewer_url(url) {
            return;
        }
        let Some(downloads_path) = &self.downloads_path else {
            return;
        };

        match self.auto_download_pdf(url, downloads_path).await {
            Ok(Some(event)) => self.record_lifecycle_event(event).await,
            Ok(None) => {}
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::pdf_auto_download_failed(
                    url,
                    error.to_string(),
                ))
                .await;
            }
        }
    }

    async fn auto_download_pdf(
        &self,
        url: &str,
        downloads_path: &Path,
    ) -> Result<Option<BrowserLifecycleEvent>, BrowserError> {
        if let Some(path) = self.cached_auto_pdf_download(url).await {
            if tokio::fs::metadata(&path).await.is_ok() {
                return Ok(None);
            }
        }

        let response = download_http_client()
            .get(url)
            .send()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(BrowserError::StateUnavailable(format!(
                "PDF download returned HTTP {}",
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        tokio::fs::create_dir_all(downloads_path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let file_name = pdf_download_filename_from_url(url);
        let path = unique_download_path(downloads_path, &file_name).await?;
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.auto_pdf_downloads
            .lock()
            .await
            .insert(url.to_owned(), path.clone());

        Ok(Some(BrowserLifecycleEvent::pdf_auto_downloaded(
            url,
            path.display().to_string(),
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or(file_name),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )))
    }

    async fn cached_auto_pdf_download(&self, url: &str) -> Option<PathBuf> {
        self.auto_pdf_downloads.lock().await.get(url).cloned()
    }

    async fn reattach_current_page(
        &self,
        stale_page: AttachedPage,
    ) -> Result<AttachedPage, BrowserError> {
        let page = match attach_to_target(&self.connection, stale_page.target_id.clone()).await {
            Ok(page) => page,
            Err(error) if is_missing_target_error(&error) => {
                attach_or_create_page(&self.connection).await?
            }
            Err(error) => return Err(error),
        };
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page.clone()).await;
        self.clear_cached_dom_state().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
        Ok(page)
    }

    async fn set_cached_dom_state(&self, dom_state: SerializedDomState) {
        *self.last_dom_state.lock().await = Some(dom_state);
    }

    async fn clear_cached_dom_state(&self) {
        *self.last_dom_state.lock().await = None;
    }

    async fn take_pending_url_policy_error(&self) -> Result<(), BrowserError> {
        if let Some(error) = self.pending_url_policy_error.lock().await.take() {
            return Err(error);
        }
        Ok(())
    }

    async fn validate_url_policy_before_navigation(&self, url: &str) -> Result<(), BrowserError> {
        match self.url_policy.validate(url) {
            Ok(()) => Ok(()),
            Err(BrowserError::NavigationBlocked { url, reason }) => {
                self.record_security_event(BrowserSecurityEvent::prevented_navigation(
                    url.clone(),
                    reason.clone(),
                ))
                .await;
                Err(BrowserError::NavigationBlocked { url, reason })
            }
            Err(error) => Err(error),
        }
    }

    async fn record_security_event(&self, event: BrowserSecurityEvent) {
        let lifecycle_event = event.lifecycle_event.clone();
        let mut events = self.security_events.lock().await;
        push_security_event(&mut events, event);
        drop(events);
        self.record_lifecycle_event(lifecycle_event).await;
    }

    async fn record_lifecycle_event(&self, event: BrowserLifecycleEvent) {
        let mut events = self.lifecycle_events.lock().await;
        push_lifecycle_event_and_publish(&mut events, &self.lifecycle_event_tx, event);
    }

    pub async fn lifecycle_events(&self) -> Vec<BrowserLifecycleEvent> {
        self.lifecycle_events.lock().await.iter().cloned().collect()
    }

    pub async fn lifecycle_adapter_events(&self) -> Vec<BrowserLifecycleAdapterEvent> {
        browser_lifecycle_adapter_events(&self.lifecycle_events().await)
    }

    pub fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    pub fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

    async fn cached_element(&self, index: u32) -> Option<CachedDomElementRef> {
        let state = self.last_dom_state.lock().await;
        let state = state.as_ref()?;
        let element = state.selector_map.get(&index)?.clone();
        let target_local_index =
            target_local_index_for_global_index(&state.selector_map, index, &element.target_id);

        Some(CachedDomElementRef {
            element,
            target_local_index,
        })
    }

    async fn evaluate_json(&self, expression: &str) -> Result<Value, BrowserError> {
        self.evaluate_json_with_options(expression, false).await
    }

    async fn evaluate_json_with_options(
        &self,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let page = self.current_page().await;
        self.evaluate_json_for_page(&page, expression, include_command_line_api)
            .await
    }

    async fn evaluate_json_for_page(
        &self,
        page: &AttachedPage,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
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
        self.evaluate_effect_for_page(&page, expression).await
    }

    async fn evaluate_effect_for_page(
        &self,
        page: &AttachedPage,
        expression: String,
    ) -> Result<(), BrowserError> {
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

    async fn page_for_element(
        &self,
        element: &DomElementRef,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        if element.target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, element.target_id.clone()).await
    }

    async fn page_for_index_fallback(
        &self,
        cached_element: Option<&CachedDomElementRef>,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        let target_id = index_fallback_target_id(&page, cached_element).to_owned();
        if target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, target_id).await
    }

    async fn resolve_element_object_id(
        &self,
        page: &AttachedPage,
        element: &DomElementRef,
    ) -> Result<String, BrowserError> {
        let params = if element.backend_node_id != 0 {
            json!({ "backendNodeId": element.backend_node_id })
        } else if let Some(node_id) = element.node_id {
            json!({ "nodeId": node_id })
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
        let page = self.page_for_element(element).await?;
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
        runtime_command_value(result, "Runtime.callFunctionOn")
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
        let root_interactive_js =
            interactive_elements_js(self.iframe_traversal, self.paint_order_filtering);
        let value = self
            .evaluate_json_for_page(&page, &root_interactive_js, true)
            .await?;
        let accessibility = self
            .accessibility_enrichment(&page)
            .await
            .unwrap_or_default();
        let _ = self
            .evaluate_effect_for_page(&page, CLEANUP_AX_REFS_JS.to_owned())
            .await;
        let root_state = dom_state_from_interactive_value(&page.target_id, &value, &accessibility)?;
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        let mut child_states = Vec::new();

        for child_page in child_pages {
            let child_interactive_js = interactive_elements_js(
                IframeTraversalConfig {
                    max_iframe_depth: self
                        .iframe_traversal
                        .remaining_same_origin_depth(child_page.depth),
                    ..self.iframe_traversal
                },
                self.paint_order_filtering,
            );
            let Ok(value) = self
                .evaluate_json_for_page(&child_page.page, &child_interactive_js, true)
                .await
            else {
                continue;
            };
            let accessibility = self
                .accessibility_enrichment(&child_page.page)
                .await
                .unwrap_or_default();
            let _ = self
                .evaluate_effect_for_page(&child_page.page, CLEANUP_AX_REFS_JS.to_owned())
                .await;
            let Ok(mut child_state) = dom_state_from_interactive_value(
                &child_page.page.target_id,
                &value,
                &accessibility,
            ) else {
                continue;
            };
            offset_dom_state_bounds(&mut child_state, child_page.offset);
            child_states.push(child_state);
        }

        Ok(merge_dom_states(root_state, child_states))
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

    async fn frame_element_infos(
        &self,
        page: &AttachedPage,
    ) -> Result<Vec<FrameElementInfo>, BrowserError> {
        let value = self
            .evaluate_json_for_page(page, FRAME_ELEMENTS_JS, false)
            .await?;
        frame_element_infos_from_value(&value)
    }

    async fn iframe_target_pages(
        &self,
        page: &AttachedPage,
        frame_infos: &[FrameElementInfo],
    ) -> Result<Vec<AttachedFramePage>, BrowserError> {
        if !self.iframe_traversal.allows_cross_origin_depth(1) {
            return Ok(Vec::new());
        }
        let targets = self
            .connection
            .command("Target.getTargets", json!({}), None)
            .await?;
        let target_infos = iframe_target_infos_from_targets(
            &targets,
            &page.target_id,
            frame_infos,
            self.iframe_traversal,
        );
        let mut pages = Vec::new();

        for target_info in target_infos {
            match attach_to_target(&self.connection, target_info.target_id).await {
                Ok(page) => pages.push(AttachedFramePage {
                    page,
                    offset: target_info.offset,
                    depth: target_info.depth,
                }),
                Err(error) if is_missing_target_error(&error) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(pages)
    }

    async fn page_text_for_page(&self, page: &AttachedPage) -> Result<String, BrowserError> {
        let value = self
            .evaluate_json_for_page(
                page,
                "(document.body ? document.body.innerText : document.documentElement.innerText || '')",
                false,
            )
            .await?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("page text".to_owned()))
    }

    async fn find_elements_for_page(
        &self,
        page: &AttachedPage,
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
            .evaluate_json_for_page(
                page,
                &format!(
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
                ),
                false,
            )
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("find elements result string".to_owned())
        })?;
        serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
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
        self.take_pending_url_policy_error().await?;

        let tabs = page_tabs(&self.connection).await?;
        let current_page = self.current_page().await;
        let mut blocked: Option<BrowserError> = None;

        for tab in tabs {
            if self.url_policy.is_allowed(&tab.url) {
                continue;
            }

            let reason = self.url_policy.block_reason(&tab.url).to_owned();
            if tab.target_id == current_page.target_id {
                let outcome = self
                    .connection
                    .command(
                        "Page.navigate",
                        json!({ "url": "about:blank" }),
                        Some(&current_page.session_id),
                    )
                    .await;
                let event = match outcome {
                    Ok(_) => BrowserSecurityEvent::reset_current(tab.url.clone(), reason.clone()),
                    Err(error) => BrowserSecurityEvent::reset_current_failed(
                        tab.url.clone(),
                        reason.clone(),
                        error.to_string(),
                    ),
                };
                self.record_security_event(event).await;
            } else {
                let outcome = self
                    .connection
                    .command(
                        "Target.closeTarget",
                        json!({ "targetId": &tab.target_id }),
                        None,
                    )
                    .await;
                match outcome {
                    Ok(_) => {
                        self.record_security_event(BrowserSecurityEvent::closed_popup(
                            tab.url.clone(),
                            reason.clone(),
                        ))
                        .await;
                    }
                    Err(error) => {
                        self.record_security_event(BrowserSecurityEvent::close_popup_failed(
                            tab.url.clone(),
                            reason.clone(),
                            error.to_string(),
                        ))
                        .await;
                        return Err(error);
                    }
                }
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

struct BrowserLifecycleWatchdog {
    handle: tokio::task::JoinHandle<()>,
}

impl BrowserLifecycleWatchdog {
    fn start(
        connection: Arc<CdpConnection>,
        lifecycle_events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
        lifecycle_event_tx: broadcast::Sender<BrowserLifecycleEvent>,
        network_request_timeout_ms: u64,
        network_activity: Arc<Mutex<NetworkActivityState>>,
        cdp_auto_pdf_download: Option<Arc<CdpAutoPdfDownloadState>>,
    ) -> Self {
        let mut events = connection.subscribe_events();
        let handle = tokio::spawn(async move {
            let mut active_network_requests = HashMap::new();
            let mut interval = tokio::time::interval(Duration::from_millis(1_000));
            let network_request_timeout = (network_request_timeout_ms > 0)
                .then(|| Duration::from_millis(network_request_timeout_ms));

            loop {
                tokio::select! {
                    event = events.recv() => {
                        match event {
                            Ok(event) => {
                                handle_lifecycle_cdp_event(
                                    &connection,
                                    &lifecycle_events,
                                    &lifecycle_event_tx,
                                    &mut active_network_requests,
                                    &network_activity,
                                    &cdp_auto_pdf_download,
                                    event,
                                )
                                .await;
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = interval.tick(), if network_request_timeout.is_some() => {
                        let timeout = network_request_timeout.expect("guarded by is_some");
                        let events = lifecycle_events_for_timed_out_network_requests(
                            &mut active_network_requests,
                            Instant::now(),
                            timeout,
                        );
                        record_lifecycle_events(&lifecycle_events, &lifecycle_event_tx, events).await;
                    }
                }
            }
        });

        Self { handle }
    }
}

impl Drop for BrowserLifecycleWatchdog {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct ActiveNetworkRequest {
    request_id: String,
    url: String,
    method: String,
    resource_type: Option<String>,
    session_id: Option<String>,
    started_at: Instant,
}

async fn handle_lifecycle_cdp_event(
    connection: &CdpConnection,
    lifecycle_events: &Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    cdp_auto_pdf_download: &Option<Arc<CdpAutoPdfDownloadState>>,
    event: CdpEvent,
) {
    match event.method.as_str() {
        "Network.requestWillBeSent" => {
            track_network_request(active_network_requests, &event);
            track_network_activity_started(network_activity, &event).await;
        }
        "Network.loadingFinished" | "Network.loadingFailed" => {
            forget_network_request(active_network_requests, &event);
            track_network_activity_finished(network_activity, &event).await;
            if event.method == "Network.loadingFinished" {
                if let Some(event) =
                    cdp_auto_pdf_lifecycle_event(connection, cdp_auto_pdf_download, &event).await
                {
                    record_lifecycle_event_in_buffer(lifecycle_events, lifecycle_event_tx, event)
                        .await;
                }
            } else if let Some(cdp_auto_pdf_download) = cdp_auto_pdf_download {
                cdp_auto_pdf_download.forget_candidate(&event).await;
            }
        }
        "Network.responseReceived" => {
            if let Some(cdp_auto_pdf_download) = cdp_auto_pdf_download {
                cdp_auto_pdf_download.observe_response(&event).await;
            }
        }
        "browser-use-rs.websocket-closed" => {
            record_lifecycle_event_in_buffer(
                lifecycle_events,
                lifecycle_event_tx,
                lifecycle_event_for_websocket_closed(&event),
            )
            .await;
        }
        "browser-use-rs.websocket-reconnecting" => {
            if let Some(event) = lifecycle_event_for_websocket_reconnecting(&event) {
                record_lifecycle_event_in_buffer(lifecycle_events, lifecycle_event_tx, event).await;
            }
        }
        "browser-use-rs.websocket-reconnected" => {
            if let Some(event) = lifecycle_event_for_websocket_reconnected(&event) {
                record_lifecycle_event_in_buffer(lifecycle_events, lifecycle_event_tx, event).await;
            }
        }
        "browser-use-rs.websocket-reconnect-failed" => {
            record_lifecycle_event_in_buffer(
                lifecycle_events,
                lifecycle_event_tx,
                lifecycle_event_for_websocket_reconnect_failed(&event),
            )
            .await;
        }
        "Target.targetCrashed" | "Inspector.targetCrashed" => {
            record_lifecycle_events(
                lifecycle_events,
                lifecycle_event_tx,
                lifecycle_events_for_target_crash(&event),
            )
            .await;
        }
        "Page.javascriptDialogOpening" => {
            let event = lifecycle_event_for_javascript_dialog(connection, &event).await;
            record_lifecycle_event_in_buffer(lifecycle_events, lifecycle_event_tx, event).await;
        }
        "Browser.downloadWillBegin" => {
            if let Some(event) = lifecycle_event_for_download_start(&event) {
                record_lifecycle_event_in_buffer(lifecycle_events, lifecycle_event_tx, event).await;
            }
        }
        "Browser.downloadProgress" => {
            record_lifecycle_events(
                lifecycle_events,
                lifecycle_event_tx,
                lifecycle_events_for_download_progress(&event),
            )
            .await;
        }
        _ => {}
    }
}

fn track_network_request(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    let Some(request) = event.params.get("request") else {
        return;
    };
    let url = request
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    active_network_requests.insert(
        request_id.to_owned(),
        ActiveNetworkRequest {
            request_id: request_id.to_owned(),
            url: url.to_owned(),
            method: request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_owned(),
            resource_type: event
                .params
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned),
            session_id: event.session_id.clone(),
            started_at: Instant::now(),
        },
    );
}

#[derive(Debug, Clone)]
struct CdpAutoPdfCandidate {
    request_id: String,
    request_key: String,
    session_id: Option<String>,
    url: String,
    file_name: String,
}

#[derive(Debug)]
struct CdpAutoPdfDownloadState {
    downloads_path: PathBuf,
    downloaded_urls: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    candidates: Mutex<BTreeMap<String, CdpAutoPdfCandidate>>,
}

impl CdpAutoPdfDownloadState {
    fn from_downloads(
        auto_download_pdfs: bool,
        downloads_path: Option<&Path>,
        downloaded_urls: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    ) -> Option<Arc<Self>> {
        if !auto_download_pdfs {
            return None;
        }
        downloads_path.map(|downloads_path| {
            Arc::new(Self {
                downloads_path: downloads_path.to_path_buf(),
                downloaded_urls,
                candidates: Mutex::new(BTreeMap::new()),
            })
        })
    }

    async fn observe_response(&self, event: &CdpEvent) {
        let Some(candidate) = cdp_auto_pdf_candidate_from_response(event) else {
            return;
        };
        self.candidates
            .lock()
            .await
            .insert(candidate.request_key.clone(), candidate);
    }

    async fn forget_candidate(&self, event: &CdpEvent) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        self.candidates.lock().await.remove(&request_key);
    }

    async fn take_finished_candidate(&self, event: &CdpEvent) -> Option<CdpAutoPdfCandidate> {
        let request_key = cdp_request_key(event)?;
        let candidate = self.candidates.lock().await.remove(&request_key)?;
        let cached_path = self
            .downloaded_urls
            .lock()
            .await
            .get(&candidate.url)
            .cloned();
        if let Some(path) = cached_path {
            if tokio::fs::metadata(&path).await.is_ok() {
                return None;
            }
            let mut downloaded_urls = self.downloaded_urls.lock().await;
            if downloaded_urls.get(&candidate.url) == Some(&path) {
                downloaded_urls.remove(&candidate.url);
            }
        }
        Some(candidate)
    }

    async fn write_candidate(
        &self,
        candidate: &CdpAutoPdfCandidate,
        bytes: &[u8],
    ) -> Result<BrowserLifecycleEvent, BrowserError> {
        tokio::fs::create_dir_all(&self.downloads_path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let path = unique_download_path(&self.downloads_path, &candidate.file_name).await?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.downloaded_urls
            .lock()
            .await
            .insert(candidate.url.clone(), path.clone());
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| candidate.file_name.clone());
        Ok(BrowserLifecycleEvent::pdf_auto_downloaded(
            &candidate.url,
            path.display().to_string(),
            file_name,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        ))
    }
}

async fn cdp_auto_pdf_lifecycle_event(
    connection: &CdpConnection,
    cdp_auto_pdf_download: &Option<Arc<CdpAutoPdfDownloadState>>,
    event: &CdpEvent,
) -> Option<BrowserLifecycleEvent> {
    let cdp_auto_pdf_download = cdp_auto_pdf_download.as_ref()?;
    let candidate = cdp_auto_pdf_download.take_finished_candidate(event).await?;
    match cdp_response_body_bytes(connection, &candidate).await {
        Ok(bytes) => match cdp_auto_pdf_download
            .write_candidate(&candidate, &bytes)
            .await
        {
            Ok(event) => Some(event),
            Err(error) => Some(BrowserLifecycleEvent::pdf_auto_download_failed(
                candidate.url,
                error.to_string(),
            )),
        },
        Err(error) => Some(BrowserLifecycleEvent::pdf_auto_download_failed(
            candidate.url,
            error.to_string(),
        )),
    }
}

async fn cdp_response_body_bytes(
    connection: &CdpConnection,
    candidate: &CdpAutoPdfCandidate,
) -> Result<Vec<u8>, BrowserError> {
    let response = connection
        .command(
            "Network.getResponseBody",
            json!({ "requestId": candidate.request_id }),
            candidate.session_id.as_deref(),
        )
        .await?;
    let body = response
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Network.getResponseBody.body".to_owned())
        })?;
    let base64_encoded = response
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if base64_encoded {
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))
    } else {
        Ok(body.as_bytes().to_vec())
    }
}

fn cdp_auto_pdf_candidate_from_response(event: &CdpEvent) -> Option<CdpAutoPdfCandidate> {
    let request_id = event.params.get("requestId").and_then(Value::as_str)?;
    let response = event.params.get("response")?;
    let url = response.get("url").and_then(Value::as_str)?.to_owned();
    let mime_type = response
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let headers = response.get("headers");
    let content_type = headers.and_then(|headers| cdp_header_value(headers, "content-type"));
    if !is_application_pdf(mime_type) && !content_type.as_deref().is_some_and(is_application_pdf) {
        return None;
    }
    let content_disposition =
        headers.and_then(|headers| cdp_header_value(headers, "content-disposition"));
    let file_name = content_disposition
        .as_deref()
        .and_then(content_disposition_filename)
        .unwrap_or_else(|| pdf_download_filename_from_url(&url));
    Some(CdpAutoPdfCandidate {
        request_id: request_id.to_owned(),
        request_key: cdp_request_key(event)?,
        session_id: event.session_id.clone(),
        url,
        file_name,
    })
}

fn cdp_request_key(event: &CdpEvent) -> Option<String> {
    let request_id = event.params.get("requestId").and_then(Value::as_str)?;
    Some(match event.session_id.as_deref() {
        Some(session_id) => format!("{session_id}:{request_id}"),
        None => format!("root:{request_id}"),
    })
}

fn cdp_header_value(headers: &Value, name: &str) -> Option<String> {
    let object = headers.as_object()?;
    object.iter().find_map(|(header_name, value)| {
        header_name.eq_ignore_ascii_case(name).then(|| match value {
            Value::String(value) => value.clone(),
            other => other.to_string(),
        })
    })
}

fn is_application_pdf(value: &str) -> bool {
    value
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("application/pdf"))
}

fn content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name == "filename*" {
            let value = value.trim().trim_matches('"');
            let encoded = value
                .rsplit_once("''")
                .map_or(value, |(_, encoded)| encoded);
            let decoded = percent_decode_str(encoded).decode_utf8_lossy();
            return Some(ensure_pdf_extension(sanitize_download_filename(&decoded)));
        }
        if name == "filename" {
            return Some(ensure_pdf_extension(sanitize_download_filename(
                value.trim().trim_matches('"'),
            )));
        }
    }
    None
}

async fn track_network_activity_started(
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    network_activity
        .lock()
        .await
        .observe_request_started(request_id, Instant::now());
}

fn forget_network_request(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    event: &CdpEvent,
) {
    if let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) {
        active_network_requests.remove(request_id);
    }
}

async fn track_network_activity_finished(
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    network_activity
        .lock()
        .await
        .observe_request_finished(request_id, Instant::now());
}

fn lifecycle_events_for_timed_out_network_requests(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    now: Instant,
    timeout: Duration,
) -> Vec<BrowserLifecycleEvent> {
    let request_ids = active_network_requests
        .iter()
        .filter(|(_, request)| now.duration_since(request.started_at) >= timeout)
        .map(|(request_id, _)| request_id.clone())
        .collect::<Vec<_>>();

    request_ids
        .into_iter()
        .filter_map(|request_id| active_network_requests.remove(&request_id))
        .map(|request| lifecycle_event_for_network_request_timeout(request, timeout))
        .collect()
}

fn lifecycle_event_for_network_request_timeout(
    request: ActiveNetworkRequest,
    timeout: Duration,
) -> BrowserLifecycleEvent {
    let timeout_seconds = format!("{:.3}", timeout.as_secs_f64());
    let mut details = BTreeMap::from([
        ("request_id".to_owned(), request.request_id.clone()),
        ("method".to_owned(), request.method.clone()),
        ("timeout_seconds".to_owned(), timeout_seconds.clone()),
    ]);
    if let Some(resource_type) = &request.resource_type {
        details.insert("resource_type".to_owned(), resource_type.clone());
    }
    if let Some(session_id) = &request.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::NetworkTimeout,
        None,
        Some(request.url.clone()),
        Some("network_request_timeout".to_owned()),
        Some(format!("request timed out after {timeout_seconds}s")),
        details,
        format!(
            "Network request {} {} timed out after {timeout_seconds}s",
            request.method, request.url
        ),
    )
}

fn lifecycle_event_for_websocket_closed(event: &CdpEvent) -> BrowserLifecycleEvent {
    let reason = event
        .params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("websocket_closed");
    let error = event
        .params
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::from([("reason".to_owned(), reason.to_owned())]);
    if let Some(error) = &error {
        details.insert("error".to_owned(), error.clone());
    }
    let message = match &error {
        Some(error) => format!("CDP websocket closed ({reason}): {error}"),
        None => format!("CDP websocket closed ({reason})"),
    };
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::BrowserStopped,
        None,
        None,
        Some(reason.to_owned()),
        error,
        details,
        message,
    )
}

fn lifecycle_event_for_websocket_reconnecting(event: &CdpEvent) -> Option<BrowserLifecycleEvent> {
    let cdp_url = event.params.get("cdp_url")?.as_str()?;
    let attempt = event.params.get("attempt")?.as_u64()? as u32;
    let max_attempts = event.params.get("max_attempts")?.as_u64()? as u32;
    Some(BrowserLifecycleEvent::browser_reconnecting(
        cdp_url,
        attempt,
        max_attempts,
    ))
}

fn lifecycle_event_for_websocket_reconnected(event: &CdpEvent) -> Option<BrowserLifecycleEvent> {
    let cdp_url = event.params.get("cdp_url")?.as_str()?;
    let attempt = event.params.get("attempt")?.as_u64()? as u32;
    let downtime_seconds = event.params.get("downtime_seconds")?.as_str()?;
    let mut lifecycle_event =
        BrowserLifecycleEvent::browser_reconnected(cdp_url, attempt, downtime_seconds);
    if let Some(generation) = event
        .params
        .get("connection_generation")
        .and_then(Value::as_u64)
    {
        lifecycle_event
            .details
            .insert("connection_generation".to_owned(), generation.to_string());
    }
    Some(lifecycle_event)
}

fn lifecycle_event_for_websocket_reconnect_failed(event: &CdpEvent) -> BrowserLifecycleEvent {
    let cdp_url = event
        .params
        .get("cdp_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let max_attempts = event
        .params
        .get("max_attempts")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let downtime_seconds = event
        .params
        .get("downtime_seconds")
        .and_then(Value::as_str)
        .unwrap_or("0.000");
    let error = event
        .params
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::from([
        ("cdp_url".to_owned(), cdp_url.to_owned()),
        ("max_attempts".to_owned(), max_attempts.to_string()),
        ("downtime_seconds".to_owned(), downtime_seconds.to_owned()),
    ]);
    if let Some(error) = &error {
        details.insert("error".to_owned(), error.clone());
    }
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::BrowserStopped,
        None,
        Some(cdp_url.to_owned()),
        Some("reconnect_failed".to_owned()),
        error,
        details,
        format!("CDP websocket failed to reconnect after {max_attempts} attempts"),
    )
}

async fn record_lifecycle_events(
    lifecycle_events: &Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    events: Vec<BrowserLifecycleEvent>,
) {
    if events.is_empty() {
        return;
    }

    let mut queue = lifecycle_events.lock().await;
    for event in events {
        push_lifecycle_event_and_publish(&mut queue, lifecycle_event_tx, event);
    }
}

async fn record_lifecycle_event_in_buffer(
    lifecycle_events: &Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    event: BrowserLifecycleEvent,
) {
    let mut queue = lifecycle_events.lock().await;
    push_lifecycle_event_and_publish(&mut queue, lifecycle_event_tx, event);
}

fn lifecycle_events_for_target_crash(event: &CdpEvent) -> Vec<BrowserLifecycleEvent> {
    let target_id = event
        .params
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::new();
    if let Some(session_id) = &event.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }
    if let Some(status) = event.params.get("status").and_then(cdp_value_to_string) {
        details.insert("status".to_owned(), status);
    }
    if let Some(error_code) = event.params.get("errorCode").and_then(cdp_value_to_string) {
        details.insert("error_code".to_owned(), error_code);
    }

    let error = target_crash_error_message(&details);
    let lifecycle_event = match target_id {
        Some(target_id) => {
            let mut event = BrowserLifecycleEvent::target_crashed(target_id, error);
            event.details = details;
            event
        }
        None => BrowserLifecycleEvent::new(
            BrowserLifecycleEventKind::TargetCrashed,
            None,
            None,
            None,
            Some(error.clone()),
            details,
            format!("Target crashed: {error}"),
        ),
    };

    vec![lifecycle_event]
}

fn target_crash_error_message(details: &BTreeMap<String, String>) -> String {
    match (details.get("status"), details.get("error_code")) {
        (Some(status), Some(error_code)) => format!("{status} ({error_code})"),
        (Some(status), None) => status.clone(),
        (None, Some(error_code)) => error_code.clone(),
        (None, None) => "Inspector target crashed".to_owned(),
    }
}

async fn lifecycle_event_for_javascript_dialog(
    connection: &CdpConnection,
    event: &CdpEvent,
) -> BrowserLifecycleEvent {
    let dialog_type = event
        .params
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("alert");
    let dialog_message = event
        .params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = event
        .params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("about:blank");
    let accepted = matches!(dialog_type, "alert" | "confirm" | "beforeunload");
    let action = if accepted { "accepted" } else { "dismissed" };
    let mut details = BTreeMap::from([
        ("dialog_type".to_owned(), dialog_type.to_owned()),
        ("dialog_message".to_owned(), dialog_message.to_owned()),
        ("action".to_owned(), action.to_owned()),
    ]);
    if let Some(frame_id) = event.params.get("frameId").and_then(Value::as_str) {
        details.insert("frame_id".to_owned(), frame_id.to_owned());
    }
    if let Some(session_id) = &event.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }

    let error = match event.session_id.as_deref() {
        Some(session_id) => connection
            .command(
                "Page.handleJavaScriptDialog",
                json!({ "accept": accepted }),
                Some(session_id),
            )
            .await
            .err()
            .map(|error| error.to_string()),
        None => Some("missing CDP session id".to_owned()),
    };

    let message = match &error {
        Some(error) => {
            format!(
                "JavaScript {dialog_type} dialog on {url} failed to be {action}: {dialog_message}: {error}"
            )
        }
        None => format!("JavaScript {dialog_type} dialog on {url} was {action}: {dialog_message}"),
    };

    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::JavaScriptDialogHandled,
        None,
        Some(url.to_owned()),
        Some(dialog_type.to_owned()),
        error,
        details,
        message,
    )
}

fn lifecycle_event_for_download_start(event: &CdpEvent) -> Option<BrowserLifecycleEvent> {
    let guid = event.params.get("guid")?.as_str()?;
    let url = event
        .params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let suggested_filename = event
        .params
        .get("suggestedFilename")
        .and_then(Value::as_str)
        .map(sanitize_download_filename)
        .unwrap_or_else(|| "download".to_owned());
    Some(BrowserLifecycleEvent::download_started(
        guid,
        url,
        suggested_filename,
    ))
}

fn lifecycle_events_for_download_progress(event: &CdpEvent) -> Vec<BrowserLifecycleEvent> {
    let Some(guid) = event.params.get("guid").and_then(Value::as_str) else {
        return Vec::new();
    };
    let received_bytes = event
        .params
        .get("receivedBytes")
        .and_then(cdp_value_to_u64)
        .unwrap_or_default();
    let total_bytes = event.params.get("totalBytes").and_then(cdp_value_to_u64);
    let state = event
        .params
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut events = vec![BrowserLifecycleEvent::download_progress(
        guid,
        received_bytes,
        total_bytes,
        state,
    )];

    if state == "completed" {
        if let Some(file_path) = event.params.get("filePath").and_then(Value::as_str) {
            let file_name = Path::new(file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_download_filename)
                .unwrap_or_else(|| "download".to_owned());
            events.push(BrowserLifecycleEvent::file_downloaded(
                guid,
                file_path,
                file_name,
                total_bytes.unwrap_or(received_bytes),
            ));
        }
    }

    events
}

fn sanitize_download_filename(name: &str) -> String {
    let cleaned = name.replace('\0', "").replace('\\', "/");
    let basename = cleaned
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("");
    if matches!(basename, "" | "." | "..") {
        "download".to_owned()
    } else {
        basename.to_owned()
    }
}

fn is_pdf_viewer_url(url: &str) -> bool {
    let path = url::Url::parse(url)
        .map(|parsed| parsed.path().to_owned())
        .unwrap_or_else(|_| url.split(['?', '#']).next().unwrap_or_default().to_owned());
    let path = path.to_ascii_lowercase();
    path.ends_with(".pdf") || path.contains("/pdf/")
}

fn pdf_download_filename_from_url(url: &str) -> String {
    let decoded_name = url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .map(|segment| percent_decode_str(segment).decode_utf8_lossy().to_string())
        })
        .unwrap_or_else(|| "download.pdf".to_owned());
    let file_name = sanitize_download_filename(&decoded_name);
    ensure_pdf_extension(file_name)
}

fn ensure_pdf_extension(file_name: String) -> String {
    if file_name.to_ascii_lowercase().ends_with(".pdf") {
        file_name
    } else {
        format!("{file_name}.pdf")
    }
}

async fn unique_download_path(
    downloads_path: &Path,
    file_name: &str,
) -> Result<PathBuf, BrowserError> {
    let file_name = sanitize_download_filename(file_name);
    let path = downloads_path.join(&file_name);
    if tokio::fs::metadata(&path).await.is_err() {
        return Ok(path);
    }

    let extension = Path::new(&file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_owned);
    let stem = Path::new(&file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("download");

    for suffix in 1_u32.. {
        let candidate_name = match &extension {
            Some(extension) if !extension.is_empty() => format!("{stem}-{suffix}.{extension}"),
            _ => format!("{stem}-{suffix}"),
        };
        let candidate = downloads_path.join(candidate_name);
        if tokio::fs::metadata(&candidate).await.is_err() {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded suffix search should always return")
}

#[cfg(test)]
fn is_path_contained(path: &Path, directory: &Path) -> bool {
    let Ok(directory) = normalize_existing_or_lexical_path(directory) else {
        return false;
    };
    let Ok(path) = normalize_existing_or_lexical_path(path) else {
        return false;
    };
    path == directory || path.starts_with(&directory)
}

#[cfg(test)]
fn normalize_existing_or_lexical_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(normalize_lexical_path(path))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn cdp_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn cdp_value_to_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| *value >= 0.0)
            .map(|value| value as u64)
    })
}

struct BrowserSecurityWatchdog {
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct LifecycleEventSink {
    events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    event_tx: broadcast::Sender<BrowserLifecycleEvent>,
}

impl LifecycleEventSink {
    async fn push(&self, event: BrowserLifecycleEvent) {
        let mut events = self.events.lock().await;
        push_lifecycle_event_and_publish(&mut events, &self.event_tx, event);
    }
}

impl BrowserSecurityWatchdog {
    async fn start(
        connection: Arc<CdpConnection>,
        page: Arc<Mutex<AttachedPage>>,
        last_dom_state: Arc<Mutex<Option<SerializedDomState>>>,
        pending_url_policy_error: Arc<Mutex<Option<BrowserError>>>,
        security_events: Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
        lifecycle_event_sink: LifecycleEventSink,
        url_policy: UrlAccessPolicy,
    ) -> Result<Option<Self>, BrowserError> {
        if url_policy.is_unrestricted() {
            return Ok(None);
        }

        let mut events = connection.subscribe_events();
        connection
            .command(
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                None,
            )
            .await?;

        let handle = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let current_page = page.lock().await.clone();
                let Some(action) =
                    url_policy_watchdog_action_for_event(&url_policy, &current_page, &event)
                else {
                    continue;
                };
                apply_url_policy_watchdog_action(
                    &connection,
                    &last_dom_state,
                    &pending_url_policy_error,
                    &security_events,
                    &lifecycle_event_sink,
                    action,
                )
                .await;
            }
        });

        Ok(Some(Self { handle }))
    }
}

impl Drop for BrowserSecurityWatchdog {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UrlPolicyWatchdogAction {
    ResetCurrent {
        session_id: String,
        url: String,
        reason: String,
    },
    CloseTarget {
        target_id: String,
        url: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserSecurityEvent {
    message: String,
    browser_error_message: Option<String>,
    closed_popup_message: Option<String>,
    lifecycle_event: BrowserLifecycleEvent,
}

impl BrowserSecurityEvent {
    fn prevented_navigation(url: String, reason: String) -> Self {
        let message =
            format!("Blocked navigation to {url} ({reason}); no browser navigation was started");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::NavigationBlocked,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            message,
            browser_error_message: None,
            closed_popup_message: None,
        }
    }

    fn reset_current(url: String, reason: String) -> Self {
        let message =
            format!("Blocked navigation to {url} ({reason}); reset current tab to about:blank");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::CurrentTargetReset,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            message,
            browser_error_message: None,
            closed_popup_message: None,
        }
    }

    fn reset_current_failed(url: String, reason: String, error: String) -> Self {
        let message = format!(
            "Failed to reset blocked navigation to {url} ({reason}) to about:blank: {error}"
        );
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::CurrentTargetResetFailed,
                None,
                Some(url),
                Some(reason),
                Some(error),
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: Some(message.clone()),
            message,
            closed_popup_message: None,
        }
    }

    fn closed_popup(url: String, reason: String) -> Self {
        let message = format!("Closed popup {url} ({reason})");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::PopupClosed,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: None,
            closed_popup_message: Some(message.clone()),
            message,
        }
    }

    fn close_popup_failed(url: String, reason: String, error: String) -> Self {
        let message = format!("Failed to close popup {url} ({reason}): {error}");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::PopupCloseFailed,
                None,
                Some(url),
                Some(reason),
                Some(error),
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: Some(message.clone()),
            closed_popup_message: None,
            message,
        }
    }

    fn from_watchdog_action(action: &UrlPolicyWatchdogAction) -> Self {
        match action {
            UrlPolicyWatchdogAction::ResetCurrent { url, reason, .. } => {
                Self::reset_current(url.clone(), reason.clone())
            }
            UrlPolicyWatchdogAction::CloseTarget { url, reason, .. } => {
                Self::closed_popup(url.clone(), reason.clone())
            }
        }
    }
}

fn url_policy_watchdog_action_for_event(
    policy: &UrlAccessPolicy,
    current_page: &AttachedPage,
    event: &CdpEvent,
) -> Option<UrlPolicyWatchdogAction> {
    match event.method.as_str() {
        "Target.targetCreated" | "Target.targetInfoChanged" => {
            let target_info = event.params.get("targetInfo")?;
            url_policy_watchdog_action_for_target_info(policy, current_page, target_info)
        }
        "Page.frameNavigated" => {
            let session_id = event.session_id.as_deref()?;
            if session_id != current_page.session_id {
                return None;
            }
            let url = event.params.get("frame")?.get("url")?.as_str()?;
            if url.is_empty() {
                return None;
            }
            if policy.is_allowed(url) {
                return None;
            }
            Some(UrlPolicyWatchdogAction::ResetCurrent {
                session_id: current_page.session_id.clone(),
                url: url.to_owned(),
                reason: policy.block_reason(url).to_owned(),
            })
        }
        _ => None,
    }
}

fn url_policy_watchdog_action_for_target_info(
    policy: &UrlAccessPolicy,
    current_page: &AttachedPage,
    target_info: &Value,
) -> Option<UrlPolicyWatchdogAction> {
    if target_info.get("type").and_then(Value::as_str) != Some("page") {
        return None;
    }

    let url = target_info.get("url")?.as_str()?;
    if url.is_empty() {
        return None;
    }
    if policy.is_allowed(url) {
        return None;
    }

    let target_id = target_info.get("targetId")?.as_str()?;
    let reason = policy.block_reason(url).to_owned();
    if target_id == current_page.target_id {
        Some(UrlPolicyWatchdogAction::ResetCurrent {
            session_id: current_page.session_id.clone(),
            url: url.to_owned(),
            reason,
        })
    } else {
        Some(UrlPolicyWatchdogAction::CloseTarget {
            target_id: target_id.to_owned(),
            url: url.to_owned(),
            reason,
        })
    }
}

async fn apply_url_policy_watchdog_action(
    connection: &CdpConnection,
    last_dom_state: &Arc<Mutex<Option<SerializedDomState>>>,
    pending_url_policy_error: &Arc<Mutex<Option<BrowserError>>>,
    security_events: &Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
    lifecycle_event_sink: &LifecycleEventSink,
    action: UrlPolicyWatchdogAction,
) {
    let event = BrowserSecurityEvent::from_watchdog_action(&action);
    let (url, reason, outcome) = match &action {
        UrlPolicyWatchdogAction::ResetCurrent {
            session_id,
            url,
            reason,
        } => (
            url.clone(),
            reason.clone(),
            connection
                .command(
                    "Page.navigate",
                    json!({ "url": "about:blank" }),
                    Some(session_id),
                )
                .await,
        ),
        UrlPolicyWatchdogAction::CloseTarget {
            target_id,
            url,
            reason,
        } => (
            url.clone(),
            reason.clone(),
            connection
                .command("Target.closeTarget", json!({ "targetId": target_id }), None)
                .await,
        ),
    };

    if let Err(error) = outcome {
        let failure_event = match &action {
            UrlPolicyWatchdogAction::ResetCurrent { .. } => {
                BrowserSecurityEvent::reset_current_failed(url, reason, error.to_string())
            }
            UrlPolicyWatchdogAction::CloseTarget { .. } => {
                BrowserSecurityEvent::close_popup_failed(url, reason, error.to_string())
            }
        };
        let lifecycle_event = failure_event.lifecycle_event.clone();
        let mut events = security_events.lock().await;
        push_security_event(&mut events, failure_event);
        drop(events);
        lifecycle_event_sink.push(lifecycle_event).await;
        return;
    }

    *last_dom_state.lock().await = None;
    {
        let lifecycle_event = event.lifecycle_event.clone();
        let mut events = security_events.lock().await;
        push_security_event(&mut events, event);
        drop(events);
        lifecycle_event_sink.push(lifecycle_event).await;
    }
    let mut pending = pending_url_policy_error.lock().await;
    if pending.is_none() {
        *pending = Some(BrowserError::NavigationBlocked { url, reason });
    }
}

fn push_security_event(events: &mut VecDeque<BrowserSecurityEvent>, event: BrowserSecurityEvent) {
    while events.len() >= MAX_SECURITY_EVENTS {
        events.pop_front();
    }
    events.push_back(event);
}

fn push_lifecycle_event(
    events: &mut VecDeque<BrowserLifecycleEvent>,
    event: BrowserLifecycleEvent,
) {
    while events.len() >= MAX_LIFECYCLE_EVENTS {
        events.pop_front();
    }
    events.push_back(event);
}

fn push_lifecycle_event_and_publish(
    events: &mut VecDeque<BrowserLifecycleEvent>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    event: BrowserLifecycleEvent,
) {
    push_lifecycle_event(events, event.clone());
    let _ = lifecycle_event_tx.send(event);
}

fn security_event_state_fields(
    events: &VecDeque<BrowserSecurityEvent>,
) -> (Option<String>, Vec<String>, Vec<String>) {
    let recent_events = (!events.is_empty()).then(|| {
        events
            .iter()
            .map(|event| event.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });
    let closed_popup_messages = events
        .iter()
        .filter_map(|event| event.closed_popup_message.clone())
        .collect();
    let browser_errors = events
        .iter()
        .filter_map(|event| event.browser_error_message.clone())
        .collect();
    (recent_events, closed_popup_messages, browser_errors)
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
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|element| !is_ax_suppressed_interactive_element(element))
        .collect::<Vec<_>>();
    let eval_root = value
        .get("eval_tree")
        .filter(|value| !value.is_null())
        .map(|value| dom_eval_node_from_value(value, accessibility))
        .transpose()?;

    let state = SerializedDomState::from_elements(elements).with_page_stats(stats);
    Ok(match eval_root {
        Some(eval_root) => state.with_eval_root(eval_root),
        None => state,
    })
}

fn dom_eval_node_from_value(
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<DomEvalNode, BrowserError> {
    let node_type = match value.get("node_type").and_then(Value::as_str) {
        Some("document_fragment") => DomEvalNodeType::DocumentFragment,
        Some("element") => DomEvalNodeType::Element,
        Some("text") => DomEvalNodeType::Text,
        Some(other) => {
            return Err(BrowserError::MissingResponseData(format!(
                "unsupported eval node type {other}"
            )));
        }
        None => {
            return Err(BrowserError::MissingResponseData(
                "eval node type".to_owned(),
            ));
        }
    };
    let ax_info = accessibility_info_for_value(value, accessibility);
    let attributes = enriched_attributes_from_value(value, ax_info);
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|child| dom_eval_node_from_value(child, accessibility))
        .collect::<Result<Vec<_>, _>>()?;
    let backend_node_id = value
        .get("backend_node_id")
        .and_then(Value::as_u64)
        .filter(|backend_node_id| *backend_node_id != 0)
        .or_else(|| ax_info.map(|info| info.backend_node_id));

    Ok(DomEvalNode {
        node_type,
        tag_name: value
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        node_value: value
            .get("node_value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        attributes,
        children,
        backend_node_id,
        should_display: value
            .get("should_display")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        excluded_by_parent: value
            .get("excluded_by_parent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_visible: value
            .get("is_visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_interactive: value
            .get("is_interactive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_scrollable: value
            .get("is_scrollable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        scroll_info: value
            .get("scroll_info")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
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

fn frame_element_infos_from_value(value: &Value) -> Result<Vec<FrameElementInfo>, BrowserError> {
    let encoded = value.as_str().ok_or_else(|| {
        BrowserError::MissingResponseData("iframe element info string".to_owned())
    })?;
    let frames: Value = serde_json::from_str(encoded)
        .map_err(|error| BrowserError::Transport(error.to_string()))?;
    let frames = frames
        .as_array()
        .ok_or_else(|| BrowserError::MissingResponseData("iframe element info array".to_owned()))?;

    Ok(frames
        .iter()
        .filter_map(|frame| {
            let url = frame.get("url")?.as_str()?.to_owned();
            let offset = FrameOffset {
                x: i32_field(frame, "x").unwrap_or_default(),
                y: i32_field(frame, "y").unwrap_or_default(),
            };
            Some(FrameElementInfo { url, offset })
        })
        .collect())
}

fn iframe_target_infos_from_targets(
    targets: &Value,
    parent_target_id: &str,
    frame_infos: &[FrameElementInfo],
    config: IframeTraversalConfig,
) -> Vec<IframeTargetInfo> {
    let depth = 1;
    if !config.allows_cross_origin_depth(depth) {
        return Vec::new();
    }
    let mut used_frames = Vec::new();
    targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("iframe"))
        .filter(|target| {
            target
                .get("parentId")
                .and_then(Value::as_str)
                .is_none_or(|parent_id| parent_id == parent_target_id)
        })
        .filter_map(|target| {
            let target_id = target.get("targetId")?.as_str()?.to_owned();
            let target_url = target.get("url").and_then(Value::as_str).unwrap_or("");
            let offset = frame_offset_for_target_url(target_url, frame_infos, &mut used_frames)?;
            Some(IframeTargetInfo {
                target_id,
                offset,
                depth,
            })
        })
        .take(config.max_iframes)
        .collect()
}

fn frame_offset_for_target_url(
    target_url: &str,
    frame_infos: &[FrameElementInfo],
    used_frames: &mut Vec<usize>,
) -> Option<FrameOffset> {
    if frame_infos.is_empty() {
        return Some(FrameOffset::default());
    }

    let index = frame_infos
        .iter()
        .enumerate()
        .find(|(index, frame)| {
            !used_frames.contains(index) && frame_url_matches(&frame.url, target_url)
        })
        .map(|(index, _)| index)?;
    used_frames.push(index);
    Some(frame_infos[index].offset)
}

fn frame_url_matches(frame_url: &str, target_url: &str) -> bool {
    if frame_url == target_url {
        return true;
    }

    let Some(frame_url) = comparable_url(frame_url) else {
        return false;
    };
    let Some(target_url) = comparable_url(target_url) else {
        return false;
    };
    frame_url == target_url
}

fn comparable_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    url.set_fragment(None);
    Some(url.to_string())
}

fn offset_dom_state_bounds(state: &mut SerializedDomState, offset: FrameOffset) {
    for element in state.selector_map.values_mut() {
        if let Some(bounds) = &mut element.bounds {
            bounds.x += offset.x;
            bounds.y += offset.y;
        }
    }
}

fn merge_dom_states(
    root_state: SerializedDomState,
    child_states: Vec<SerializedDomState>,
) -> SerializedDomState {
    if child_states.is_empty() {
        return root_state;
    }

    let mut root_state = root_state;
    let mut page_stats = root_state.page_stats;
    let mut eval_root = root_state.eval_root.take();
    let mut elements = dom_state_elements(root_state);
    for mut child_state in child_states {
        add_dom_page_stats(&mut page_stats, child_state.page_stats);
        if let Some(child_eval_root) = child_state.eval_root.take() {
            attach_child_eval_root(&mut eval_root, child_eval_root);
        }
        elements.extend(dom_state_elements(child_state));
    }

    for (index, element) in elements.iter_mut().enumerate() {
        element.index = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }

    let state = SerializedDomState::from_elements(elements).with_page_stats(page_stats);
    match eval_root {
        Some(eval_root) => state.with_eval_root(eval_root),
        None => state,
    }
}

fn dom_state_elements(state: SerializedDomState) -> Vec<DomElementRef> {
    state.selector_map.into_values().collect()
}

fn target_local_index_for_global_index(
    selector_map: &BTreeMap<u32, DomElementRef>,
    global_index: u32,
    target_id: &str,
) -> u32 {
    let mut local_index = 0_u32;
    for (candidate_index, element) in selector_map {
        if element.target_id != target_id {
            continue;
        }
        local_index = local_index.saturating_add(1);
        if *candidate_index == global_index {
            return local_index;
        }
    }

    global_index
}

fn index_fallback_target_id<'a>(
    current_page: &'a AttachedPage,
    cached_element: Option<&'a CachedDomElementRef>,
) -> &'a str {
    cached_element
        .map(|cached| cached.element.target_id.as_str())
        .filter(|target_id| !target_id.is_empty())
        .unwrap_or(current_page.target_id.as_str())
}

fn attach_child_eval_root(eval_root: &mut Option<DomEvalNode>, child_eval_root: DomEvalNode) {
    let Some(root) = eval_root else {
        *eval_root = Some(child_eval_root);
        return;
    };

    let mut child_roots = VecDeque::from([child_eval_root]);
    attach_eval_roots_to_iframes(root, &mut child_roots);
    while let Some(child_root) = child_roots.pop_front() {
        root.children
            .extend(eval_iframe_content_children(child_root));
    }
}

fn attach_eval_roots_to_iframes(node: &mut DomEvalNode, child_roots: &mut VecDeque<DomEvalNode>) {
    if child_roots.is_empty() {
        return;
    }
    if node.node_type == DomEvalNodeType::Element
        && matches!(node.tag_name.as_str(), "iframe" | "frame")
        && node.children.is_empty()
        && let Some(child_root) = child_roots.pop_front()
    {
        node.children
            .extend(eval_iframe_content_children(child_root));
        return;
    }

    for child in &mut node.children {
        attach_eval_roots_to_iframes(child, child_roots);
        if child_roots.is_empty() {
            return;
        }
    }
}

fn eval_iframe_content_children(child_root: DomEvalNode) -> Vec<DomEvalNode> {
    if child_root.node_type == DomEvalNodeType::Element && child_root.tag_name == "html" {
        if let Some(body) = child_root
            .children
            .into_iter()
            .find(|child| child.node_type == DomEvalNodeType::Element && child.tag_name == "body")
        {
            return body.children;
        }
        return Vec::new();
    }
    if child_root.node_type == DomEvalNodeType::Element && child_root.tag_name == "body" {
        return child_root.children;
    }
    vec![child_root]
}

fn add_dom_page_stats(total: &mut DomPageStats, next: DomPageStats) {
    total.links = total.links.saturating_add(next.links);
    total.iframes = total.iframes.saturating_add(next.iframes);
    total.shadow_open = total.shadow_open.saturating_add(next.shadow_open);
    total.shadow_closed = total.shadow_closed.saturating_add(next.shadow_closed);
    total.scroll_containers = total
        .scroll_containers
        .saturating_add(next.scroll_containers);
    total.images = total.images.saturating_add(next.images);
    total.interactive_elements = total
        .interactive_elements
        .saturating_add(next.interactive_elements);
    total.total_elements = total.total_elements.saturating_add(next.total_elements);
    total.text_chars = total.text_chars.saturating_add(next.text_chars);
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
    let ax_info = accessibility_info_for_value(value, accessibility);
    let attributes = enriched_attributes_from_value(value, ax_info);
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
            let mut properties = ax_node_properties(node);
            for field in ["value", "description"] {
                if let Some(value) = ax_node_field_to_string(node, field) {
                    properties.entry(field.to_owned()).or_insert(value);
                }
            }
            Some((
                backend_node_id,
                AccessibilityNodeInfo {
                    backend_node_id,
                    node_id: None,
                    role: ax_property_value(node, "role").map(str::to_owned),
                    name: ax_property_value(node, "name").map(str::to_owned),
                    properties,
                },
            ))
        })
        .collect()
}

fn accessibility_info_for_value<'a>(
    value: &Value,
    accessibility: &'a BTreeMap<String, AccessibilityNodeInfo>,
) -> Option<&'a AccessibilityNodeInfo> {
    value
        .get("ax_ref")
        .and_then(Value::as_str)
        .and_then(|ax_ref| accessibility.get(ax_ref))
}

fn enriched_attributes_from_value(
    value: &Value,
    ax_info: Option<&AccessibilityNodeInfo>,
) -> BTreeMap<String, String> {
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

    if let Some(ax_info) = ax_info {
        attributes.extend(ax_info.properties.clone());
        if let Some(name) = nonempty_value(ax_info.name.as_deref()) {
            attributes.insert("ax_name".to_owned(), name.to_owned());
        }
        if let Some(description) = ax_info
            .properties
            .get("description")
            .and_then(|value| nonempty_value(Some(value)))
        {
            attributes.insert("ax_description".to_owned(), description.to_owned());
        }
    }

    attributes
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
    ax_value_to_string(property.get("value")?)
}

fn ax_node_field_to_string(node: &Value, field: &str) -> Option<String> {
    ax_value_to_string(node.get(field)?)
}

fn ax_value_to_string(value: &Value) -> Option<String> {
    match value.get("value")? {
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => nonempty_value(Some(value)).map(str::to_owned),
        _ => None,
    }
}

fn nonempty_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn is_ax_suppressed_interactive_element(element: &DomElementRef) -> bool {
    ["disabled", "hidden"].into_iter().any(|attribute| {
        element
            .attributes
            .get(attribute)
            .is_some_and(|value| is_truthy_accessibility_value(value))
    })
}

fn is_truthy_accessibility_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

fn is_useful_ax_role(role: &str) -> bool {
    !matches!(role, "generic" | "none" | "presentation" | "StaticText")
}

fn should_fallback_to_index_traversal(error: &BrowserError) -> bool {
    match error {
        BrowserError::MissingResponseData(message) => message.contains("cached element node id"),
        BrowserError::CommandFailed { method, message } => {
            (method == "DOM.resolveNode"
                && (message.contains("No node")
                    || message.contains("Could not find")
                    || message.contains("Invalid remote object id")))
                || (method == "Runtime.callFunctionOn"
                    && message.contains("cached element is detached from DOM"))
        }
        _ => false,
    }
}

fn is_missing_target_error(error: &BrowserError) -> bool {
    matches!(
        error,
        BrowserError::CommandFailed { method, message }
            if method == "Target.attachToTarget" && message.contains("No target with given id found")
    )
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
    runtime_command_value(result, "Runtime.evaluate")
}

fn runtime_command_value(result: Value, method: &str) -> Result<Value, BrowserError> {
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

fn render_runtime_evaluate_result(result: &Value) -> Result<String, BrowserError> {
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

    connection.register_attached_session(&session_id).await;
    connection
        .command("Page.enable", json!({}), Some(&session_id))
        .await?;
    connection
        .command("Network.enable", json!({}), Some(&session_id))
        .await?;

    Ok(AttachedPage {
        target_id,
        session_id,
    })
}

fn viewport_emulation_params(config: ViewportEmulationConfig) -> Option<Value> {
    config.viewport.map(|viewport| {
        json!({
            "width": viewport.width,
            "height": viewport.height,
            "deviceScaleFactor": config.device_scale_factor,
            "mobile": false,
        })
    })
}

async fn apply_viewport_emulation_for_page(
    connection: &CdpConnection,
    page: &AttachedPage,
    config: ViewportEmulationConfig,
) -> Result<(), BrowserError> {
    let Some(params) = viewport_emulation_params(config) else {
        return Ok(());
    };
    connection
        .command(
            "Emulation.setDeviceMetricsOverride",
            params,
            Some(&page.session_id),
        )
        .await
        .map(|_| ())
}

fn browser_permission_grant_params(permissions: &[String]) -> Option<Value> {
    (!permissions.is_empty()).then(|| json!({ "permissions": permissions }))
}

async fn grant_browser_permissions(
    connection: &CdpConnection,
    permissions: &[String],
) -> Option<BrowserLifecycleEvent> {
    let params = browser_permission_grant_params(permissions)?;
    match connection
        .command("Browser.grantPermissions", params, None)
        .await
    {
        Ok(_) => None,
        Err(error) => Some(BrowserLifecycleEvent::permissions_grant_failed(
            permissions,
            error.to_string(),
        )),
    }
}

async fn enable_browser_download_events(
    connection: &CdpConnection,
    downloads_path: &Path,
) -> Result<(), BrowserError> {
    tokio::fs::create_dir_all(downloads_path)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    let downloads_path = downloads_path.display().to_string();
    connection
        .command(
            "Browser.setDownloadBehavior",
            json!({
                "behavior": "allow",
                "downloadPath": downloads_path,
                "eventsEnabled": true,
            }),
            None,
        )
        .await
        .map(|_| ())
}

const ORIGIN_STORAGE_STATE_JS: &str = r#"
(() => {
  const origin = window.location && window.location.origin;
  if (!origin || origin === 'null') return null;
  const entries = (storage) => {
    const out = [];
    for (let index = 0; index < storage.length; index += 1) {
      const name = storage.key(index);
      if (name === null) continue;
      out.push({ name, value: storage.getItem(name) || '' });
    }
    return out;
  };
  return {
    origin,
    localStorage: entries(window.localStorage),
    sessionStorage: entries(window.sessionStorage),
  };
})()
"#;

async fn browser_storage_state(
    connection: &CdpConnection,
    page: Option<&AttachedPage>,
) -> Result<Value, BrowserError> {
    let cookies = connection
        .command("Network.getAllCookies", json!({}), None)
        .await?
        .get("cookies")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut state = json!({
        "cookies": cookies,
        "origins": [],
    });
    if let Some(page) = page {
        state["origins"] = Value::Array(origin_storage_states(connection, page).await?);
    }
    Ok(state)
}

async fn origin_storage_states(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<Vec<Value>, BrowserError> {
    let mut states = BTreeMap::new();
    if let Some(origin_state) = current_origin_storage_state(connection, page).await? {
        upsert_origin_storage_state(&mut states, origin_state);
    }

    if let Ok(origins) = frame_security_origins(connection, page).await {
        let _ = connection
            .command("DOMStorage.enable", json!({}), Some(&page.session_id))
            .await;
        for origin in origins {
            if let Some(origin_state) =
                dom_storage_origin_state(connection, page, origin.as_str()).await
            {
                upsert_origin_storage_state(&mut states, origin_state);
            }
        }
    }

    Ok(states.into_values().collect())
}

async fn current_origin_storage_state(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<Option<Value>, BrowserError> {
    let result = connection
        .command(
            "Runtime.evaluate",
            runtime_evaluate_params(ORIGIN_STORAGE_STATE_JS, false),
            Some(&page.session_id),
        )
        .await?;
    let value = runtime_evaluate_value(result)?;
    if value.is_null() || !origin_storage_has_items(&value) {
        return Ok(None);
    }
    Ok(Some(value))
}

fn origin_storage_has_items(origin_state: &Value) -> bool {
    origin_state
        .get("localStorage")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || origin_state
            .get("sessionStorage")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
}

fn upsert_origin_storage_state(states: &mut BTreeMap<String, Value>, origin_state: Value) {
    let Some(origin) = origin_state.get("origin").and_then(Value::as_str) else {
        return;
    };
    if !origin_storage_has_items(&origin_state) {
        return;
    }

    states
        .entry(origin.to_owned())
        .and_modify(|existing| {
            *existing = merge_origin_storage_states(existing, &origin_state);
        })
        .or_insert(origin_state);
}

fn merge_origin_storage_states(existing: &Value, incoming: &Value) -> Value {
    let origin = incoming
        .get("origin")
        .and_then(Value::as_str)
        .or_else(|| existing.get("origin").and_then(Value::as_str))
        .unwrap_or_default();
    json!({
        "origin": origin,
        "localStorage": merge_storage_item_arrays(
            existing.get("localStorage"),
            incoming.get("localStorage"),
        ),
        "sessionStorage": merge_storage_item_arrays(
            existing.get("sessionStorage"),
            incoming.get("sessionStorage"),
        ),
    })
}

fn merge_storage_item_arrays(first: Option<&Value>, second: Option<&Value>) -> Value {
    let mut items = BTreeMap::new();
    for item in first
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(second.and_then(Value::as_array).into_iter().flatten())
    {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let value = item
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        items.insert(name.to_owned(), value.to_owned());
    }

    Value::Array(
        items
            .into_iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect(),
    )
}

async fn frame_security_origins(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<BTreeSet<String>, BrowserError> {
    let result = connection
        .command("Page.getFrameTree", json!({}), Some(&page.session_id))
        .await?;
    Ok(frame_security_origins_from_result(&result))
}

fn frame_security_origins_from_result(result: &Value) -> BTreeSet<String> {
    let mut origins = BTreeSet::new();
    if let Some(frame_tree) = result.get("frameTree") {
        collect_frame_security_origins(frame_tree, &mut origins);
    }
    origins
}

fn collect_frame_security_origins(frame_tree: &Value, origins: &mut BTreeSet<String>) {
    if let Some(frame) = frame_tree.get("frame") {
        if let Some(origin) = security_origin_for_frame(frame) {
            origins.insert(origin);
        }
    }
    for child in frame_tree
        .get("childFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_frame_security_origins(child, origins);
    }
}

fn security_origin_for_frame(frame: &Value) -> Option<String> {
    frame
        .get("securityOrigin")
        .and_then(Value::as_str)
        .and_then(normalize_http_origin)
        .or_else(|| {
            frame
                .get("url")
                .and_then(Value::as_str)
                .and_then(http_origin_for_url)
        })
}

fn normalize_http_origin(origin: &str) -> Option<String> {
    let origin = origin.trim();
    if origin.starts_with("http://") || origin.starts_with("https://") {
        Some(origin.trim_end_matches('/').to_owned())
    } else {
        None
    }
}

fn http_origin_for_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

async fn dom_storage_origin_state(
    connection: &CdpConnection,
    page: &AttachedPage,
    origin: &str,
) -> Option<Value> {
    let local_storage = dom_storage_items(connection, page, origin, true)
        .await
        .unwrap_or_default();
    let session_storage = dom_storage_items(connection, page, origin, false)
        .await
        .unwrap_or_default();
    let origin_state = json!({
        "origin": origin,
        "localStorage": local_storage,
        "sessionStorage": session_storage,
    });
    origin_storage_has_items(&origin_state).then_some(origin_state)
}

async fn dom_storage_items(
    connection: &CdpConnection,
    page: &AttachedPage,
    origin: &str,
    is_local_storage: bool,
) -> Result<Vec<Value>, BrowserError> {
    let result = connection
        .command(
            "DOMStorage.getDOMStorageItems",
            json!({
                "storageId": {
                    "securityOrigin": origin,
                    "isLocalStorage": is_local_storage,
                }
            }),
            Some(&page.session_id),
        )
        .await?;
    Ok(dom_storage_entries_to_items(result.get("entries")))
}

fn dom_storage_entries_to_items(entries: Option<&Value>) -> Vec<Value> {
    let mut items = BTreeMap::new();
    for entry in entries.and_then(Value::as_array).into_iter().flatten() {
        let Some(pair) = entry.as_array() else {
            continue;
        };
        let Some(name) = pair.first().and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = pair.get(1).and_then(Value::as_str) else {
            continue;
        };
        items.insert(name.to_owned(), value.to_owned());
    }
    items
        .into_iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect()
}

async fn load_browser_storage_state(
    connection: &CdpConnection,
    path: &Path,
) -> Result<Value, BrowserError> {
    if !path.exists() {
        return Ok(json!({
            "cookies": [],
            "origins": [],
        }));
    }
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    let storage_state: Value = serde_json::from_str(&text)
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    if let Some(cookies) = storage_state.get("cookies").and_then(Value::as_array) {
        if !cookies.is_empty() {
            connection
                .command("Network.setCookies", json!({ "cookies": cookies }), None)
                .await?;
        }
    }
    Ok(storage_state)
}

async fn apply_origin_storage_state(
    connection: &CdpConnection,
    page: &AttachedPage,
    storage_state: &Value,
) -> Result<(), BrowserError> {
    let origins = storage_state
        .get("origins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for origin_state in origins {
        let Some(source) = origin_storage_apply_script(&origin_state) else {
            continue;
        };
        connection
            .command(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": source }),
                Some(&page.session_id),
            )
            .await?;
        connection
            .command(
                "Runtime.evaluate",
                runtime_evaluate_params(&source, false),
                Some(&page.session_id),
            )
            .await?;
    }
    Ok(())
}

fn origin_storage_apply_script(origin_state: &Value) -> Option<String> {
    let origin = origin_state.get("origin")?.as_str()?;
    let local_storage = storage_items_object(origin_state.get("localStorage"));
    let session_storage = storage_items_object(origin_state.get("sessionStorage"));
    if storage_items_are_empty(&local_storage) && storage_items_are_empty(&session_storage) {
        return None;
    }
    Some(format!(
        r#"(() => {{
  const expectedOrigin = {origin_json};
  if (!window.location || window.location.origin !== expectedOrigin) return;
  const localItems = {local_json};
  for (const [name, value] of Object.entries(localItems)) window.localStorage.setItem(name, value);
  const sessionItems = {session_json};
  for (const [name, value] of Object.entries(sessionItems)) window.sessionStorage.setItem(name, value);
}})()"#,
        origin_json = serde_json::to_string(origin).ok()?,
        local_json = local_storage,
        session_json = session_storage,
    ))
}

fn storage_items_are_empty(value: &Value) -> bool {
    value
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(true)
}

fn storage_items_object(items: Option<&Value>) -> Value {
    let mut object = serde_json::Map::new();
    for item in items.and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let value = item
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        object.insert(name.to_owned(), Value::String(value.to_owned()));
    }
    Value::Object(object)
}

async fn write_storage_state(path: &Path, storage_state: &Value) -> Result<(), BrowserError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    }
    let text = serde_json::to_string_pretty(storage_state)
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    tokio::fs::write(path, text)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))
}

fn storage_state_counts(storage_state: &Value) -> (usize, usize) {
    let cookies_count = storage_state
        .get("cookies")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let origins_count = storage_state
        .get("origins")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    (cookies_count, origins_count)
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
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.enforce_open_tab_url_policy().await?;
        self.wait_for_page_load_settle().await;
        let (url, title) = self.page_location().await?;
        let is_pdf_viewer = is_pdf_viewer_url(&url);
        if is_pdf_viewer {
            self.auto_download_pdf_if_needed(&url).await;
        }
        let page_info = self.page_info().await?;
        let dom_state = self.dom_state().await?;
        self.set_cached_dom_state(dom_state.clone()).await;
        let pagination_buttons = detect_pagination_buttons(&dom_state);
        let current_page = self.current_page().await;
        let tabs = page_tabs(&self.connection).await?;
        let (recent_events, closed_popup_messages, browser_errors) = {
            let events = self.security_events.lock().await;
            security_event_state_fields(&events)
        };
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
            browser_errors,
            is_pdf_viewer,
            recent_events,
            pending_network_requests: vec![],
            pagination_buttons,
            closed_popup_messages,
        })
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.validate_url_policy_before_navigation(url).await?;
        if new_tab {
            let target_id = create_target(&self.connection, url).await?;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_created(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            let page = match attach_to_target(&self.connection, target_id.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id.clone(),
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                    return Err(error);
                }
            };
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id.clone()))
                .await;
            self.clear_cached_dom_state().await;
            let result = self.enforce_url_policy_after_settle().await;
            if result.is_ok() {
                self.wait_for_page_load_settle().await;
            }
            match &result {
                Ok(()) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                        target_id,
                        url.to_owned(),
                    ))
                    .await;
                }
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id,
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                }
            }
            return result;
        }

        let page = self.current_page().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
            page.target_id.clone(),
            url.to_owned(),
        ))
        .await;
        let navigate = self.connection.command(
            "Page.navigate",
            json!({
                "url": url,
            }),
            Some(&page.session_id),
        );
        let navigate_result = if self.navigation_timeout_ms == 0 {
            navigate.await
        } else {
            match tokio::time::timeout(Duration::from_millis(self.navigation_timeout_ms), navigate)
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    let timeout_seconds =
                        format!("{:.3}", self.navigation_timeout_ms as f64 / 1000.0);
                    self.record_lifecycle_event(BrowserLifecycleEvent::network_timeout(
                        page.target_id.clone(),
                        url.to_owned(),
                        timeout_seconds,
                    ))
                    .await;
                    return Err(BrowserError::NavigationFailed(format!(
                        "Page.navigate timed out after {}ms for {url}",
                        self.navigation_timeout_ms
                    )));
                }
            }
        };
        if let Err(error) = navigate_result {
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                page.target_id.clone(),
                url.to_owned(),
                error.to_string(),
            ))
            .await;
            return Err(error);
        }
        self.clear_cached_dom_state().await;
        let result = self.enforce_url_policy_after_settle().await;
        if result.is_ok() {
            self.wait_for_page_load_settle().await;
        }
        match &result {
            Ok(()) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                    page.target_id,
                    url.to_owned(),
                ))
                .await;
            }
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                    page.target_id,
                    url.to_owned(),
                    error.to_string(),
                ))
                .await;
            }
        }
        result
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
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page).await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
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
        self.record_lifecycle_event(BrowserLifecycleEvent::target_closed(target_id.clone()))
            .await;

        if self.current_page().await.target_id == target_id {
            let page = attach_or_create_page(&self.connection).await?;
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
                .await;
        }
        self.clear_cached_dom_state().await;

        self.enforce_open_tab_url_policy().await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(
                    &cached.element,
                    element_action_function_js(CLICK_ELEMENT_ACTION_JS),
                )
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, click_element_js(fallback_index))
            .await?;
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
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_action_function_js(&action))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
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
            let cached_element = self.cached_element(index).await;
            if let Some(cached) = cached_element.as_ref() {
                match self
                    .call_element_function(&cached.element, element_action_function_js(&action))
                    .await
                {
                    Ok(()) => return self.enforce_url_policy_after_settle().await,
                    Err(error) if should_fallback_to_index_traversal(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            let page = self
                .page_for_index_fallback(cached_element.as_ref())
                .await?;
            let fallback_index = cached_element
                .as_ref()
                .map(|cached| cached.target_local_index)
                .unwrap_or(index);
            self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
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
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function_value(
                    &cached.element,
                    element_function_js(DROPDOWN_OPTIONS_BODY_JS),
                )
                .await
            {
                Ok(value) => return parse_dropdown_options_value(value),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        let value = self
            .evaluate_json_for_page(&page, &dropdown_options_js(fallback_index), false)
            .await?;
        parse_dropdown_options_value(value)
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        let body = select_dropdown_option_body_js(text)?;
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, select_dropdown_option_js(fallback_index, text)?)
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let mut texts = Vec::new();
        let root_text = self.page_text_for_page(&page).await?;
        if !root_text.trim().is_empty() {
            texts.push(root_text);
        }
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let Ok(text) = self.page_text_for_page(&child_page.page).await else {
                continue;
            };
            if !text.trim().is_empty() {
                texts.push(text);
            }
        }
        Ok(texts.join("\n"))
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let page = self.current_page().await;
        let mut elements = self
            .find_elements_for_page(&page, selector, attributes, max_results, include_text)
            .await?;
        if elements.len() >= max_results {
            return Ok(elements);
        }

        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let remaining = max_results.saturating_sub(elements.len());
            if remaining == 0 {
                break;
            }
            let Ok(mut child_elements) = self
                .find_elements_for_page(
                    &child_page.page,
                    selector,
                    attributes,
                    remaining,
                    include_text,
                )
                .await
            else {
                continue;
            };
            elements.append(&mut child_elements);
        }

        Ok(elements)
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
        let mut marked_cached_element = None;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&mark_upload_body))
                .await
            {
                Ok(()) => marked_cached_element = Some(cached.clone()),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = if let Some(cached) = marked_cached_element.as_ref() {
            self.page_for_element(&cached.element).await?
        } else {
            self.page_for_index_fallback(cached_element.as_ref())
                .await?
        };
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        if marked_cached_element.is_none() {
            self.evaluate_effect_for_page(
                &page,
                element_eval_js(fallback_index, &mark_upload_body),
            )
            .await?;
        }

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
        if let Some(cached) = marked_cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(finish_upload_body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect_for_page(&page, element_eval_js(fallback_index, finish_upload_body))
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
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::closed()
    }

    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

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
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        self.as_ref().subscribe_lifecycle_events()
    }

    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        self.as_ref().subscribe_lifecycle_adapter_events()
    }

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

    fn cloud_browser_response_json(id: &str, status: &str) -> Value {
        json!({
            "id": id,
            "status": status,
            "liveUrl": format!("https://cloud.browser-use.com/live/{id}"),
            "cdpUrl": format!("wss://cdp.browser-use.com/devtools/browser/{id}"),
            "timeoutAt": "2026-05-18T20:00:00Z",
            "startedAt": "2026-05-18T19:00:00Z",
            "finishedAt": null
        })
    }

    async fn cloud_test_server(
        responses: Vec<(u16, Value)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cloud test server");
        let addr = listener.local_addr().expect("cloud test server addr");
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("accept cloud request");
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.expect("read cloud request");
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if http_request_complete(&buffer) {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&buffer).to_string();
                requests.push(request);
                let body = body.to_string();
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write cloud response");
            }
            requests
        });
        (format!("http://{addr}"), handle)
    }

    async fn pdf_download_test_server(
        body: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<usize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind PDF test server");
        let addr = listener.local_addr().expect("PDF test server addr");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept PDF request");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await.expect("read PDF request");
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/pdf\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write PDF response headers");
            stream
                .write_all(body)
                .await
                .expect("write PDF response body");
            1
        });
        (format!("http://{addr}/docs/report.pdf"), handle)
    }

    fn test_session_for_pdf_downloads(
        downloads_path: Option<PathBuf>,
        auto_download_pdfs: bool,
    ) -> CdpBrowserSession {
        let (request_tx, _request_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(16);
        let connection = Arc::new(CdpConnection {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            session_generations: Arc::new(Mutex::new(HashMap::new())),
        });
        let (lifecycle_event_tx, _) = broadcast::channel(16);
        CdpBrowserSession {
            connection,
            page: Arc::new(Mutex::new(AttachedPage {
                target_id: "target-1".to_owned(),
                session_id: "session-1".to_owned(),
            })),
            last_dom_state: Arc::new(Mutex::new(None)),
            pending_url_policy_error: Arc::new(Mutex::new(None)),
            security_events: Arc::new(Mutex::new(VecDeque::new())),
            lifecycle_events: Arc::new(Mutex::new(VecDeque::new())),
            lifecycle_event_tx,
            url_policy: UrlAccessPolicy::from_profile(&BrowserProfile::default()),
            iframe_traversal: IframeTraversalConfig::from_profile(&BrowserProfile::default()),
            paint_order_filtering: default_paint_order_filtering(),
            viewport_emulation: ViewportEmulationConfig::from_profile(&BrowserProfile::default()),
            page_load_wait: PageLoadWaitConfig::from_profile(&BrowserProfile::default()),
            network_activity: Arc::new(Mutex::new(NetworkActivityState::new(Instant::now()))),
            downloads_path,
            auto_download_pdfs,
            auto_pdf_downloads: Arc::new(Mutex::new(BTreeMap::new())),
            storage_state_path: None,
            navigation_timeout_ms: default_navigation_timeout_ms(),
            _lifecycle_watchdog: BrowserLifecycleWatchdog {
                handle: tokio::spawn(async {}),
            },
            _security_watchdog: None,
            _launched_browser: None,
            _downloads_dir: None,
        }
    }

    fn http_request_complete(buffer: &[u8]) -> bool {
        let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        buffer.len() >= header_end + content_length
    }

    fn request_body(request: &str) -> Value {
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request body separator");
        serde_json::from_str(body).expect("request body json")
    }

    fn request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        request.lines().find_map(|line| {
            line.split_once(':').and_then(|(header_name, value)| {
                header_name.eq_ignore_ascii_case(name).then(|| value.trim())
            })
        })
    }

    #[derive(Debug, Clone)]
    struct RecordedCdpCommand {
        method: String,
        params: Value,
        session_id: Option<String>,
    }

    async fn cdp_command_test_server(
        grant_error: Option<&'static str>,
        expected_requests: usize,
    ) -> (
        DevToolsEndpoint,
        tokio::task::JoinHandle<Vec<RecordedCdpCommand>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cdp test server");
        let addr = listener.local_addr().expect("cdp test server addr");
        let endpoint = DevToolsEndpoint {
            http_url: format!("http://{addr}"),
            websocket_url: format!("ws://{addr}/devtools/browser/test"),
        };
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept cdp websocket");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept websocket handshake");
            let mut commands = Vec::new();

            for _ in 0..expected_requests {
                let Some(message) = websocket.next().await else {
                    break;
                };
                let message = message.expect("cdp websocket message");
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).expect("utf8 cdp"),
                    _ => continue,
                };
                let payload: Value = serde_json::from_str(&text).expect("cdp request json");
                let id = payload.get("id").and_then(Value::as_u64).expect("cdp id");
                let method = payload
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("cdp method");
                let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                commands.push(RecordedCdpCommand {
                    method: method.to_owned(),
                    params,
                    session_id: payload
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });

                let response = cdp_command_test_response(id, method, grant_error);
                websocket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .expect("send cdp response");
            }

            commands
        });
        (endpoint, handle)
    }

    #[allow(clippy::result_large_err)]
    async fn cdp_command_header_test_server(
        expected_requests: usize,
    ) -> (
        DevToolsEndpoint,
        tokio::task::JoinHandle<(Vec<RecordedCdpCommand>, BTreeMap<String, String>)>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cdp header test server");
        let addr = listener.local_addr().expect("cdp header test server addr");
        let endpoint = DevToolsEndpoint {
            http_url: format!("http://{addr}"),
            websocket_url: format!("ws://{addr}/devtools/browser/test"),
        };
        let handle = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .expect("accept cdp header websocket");
            let handshake_headers =
                Arc::new(std::sync::Mutex::new(BTreeMap::<String, String>::new()));
            let captured_headers = handshake_headers.clone();
            let mut websocket = tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    let mut headers = captured_headers
                        .lock()
                        .expect("capture websocket handshake headers");
                    for (name, value) in request.headers() {
                        if let Ok(value) = value.to_str() {
                            headers.insert(name.as_str().to_ascii_lowercase(), value.to_owned());
                        }
                    }
                    Ok(response)
                },
            )
            .await
            .expect("accept websocket handshake");
            let mut commands = Vec::new();

            for _ in 0..expected_requests {
                let Some(message) = websocket.next().await else {
                    break;
                };
                let message = message.expect("cdp websocket message");
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).expect("utf8 cdp"),
                    _ => continue,
                };
                let payload: Value = serde_json::from_str(&text).expect("cdp request json");
                let id = payload.get("id").and_then(Value::as_u64).expect("cdp id");
                let method = payload
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("cdp method");
                let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                commands.push(RecordedCdpCommand {
                    method: method.to_owned(),
                    params,
                    session_id: payload
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });

                let response = cdp_command_test_response(id, method, None);
                websocket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .expect("send cdp response");
            }

            let headers = handshake_headers
                .lock()
                .expect("read websocket handshake headers")
                .clone();
            (commands, headers)
        });
        (endpoint, handle)
    }

    fn cdp_command_test_response(
        id: u64,
        method: &str,
        grant_error: Option<&'static str>,
    ) -> Value {
        if method == "Browser.grantPermissions" {
            if let Some(message) = grant_error {
                return json!({
                    "id": id,
                    "error": {
                        "message": message
                    }
                });
            }
        }

        let result = match method {
            "Target.getTargets" => json!({
                "targetInfos": [{
                    "targetId": "target-1",
                    "type": "page",
                    "url": "about:blank"
                }]
            }),
            "Target.attachToTarget" => json!({
                "sessionId": "session-1"
            }),
            "Browser.grantPermissions"
            | "Browser.setDownloadBehavior"
            | "Page.enable"
            | "Network.enable"
            | "Emulation.setDeviceMetricsOverride" => json!({}),
            "Network.getResponseBody" => json!({
                "body": base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 cdp body"),
                "base64Encoded": true
            }),
            other => panic!("unexpected CDP method {other}"),
        };
        json!({
            "id": id,
            "result": result
        })
    }

    fn arg_index(args: &[String], expected: &str) -> usize {
        args.iter()
            .position(|arg| arg == expected)
            .unwrap_or_else(|| panic!("missing launch arg {expected} in {args:?}"))
    }

    #[test]
    fn default_profile_uses_headless_chrome_args() {
        let profile = BrowserProfile::default();
        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(plan.args.contains(&"--window-size=1280,720".to_owned()));
        assert!(plan.args.contains(&"--window-position=0,0".to_owned()));
        assert!(!profile.devtools);
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--auto-open-devtools-for-tabs")
        );
        assert_eq!(profile.window_size, None);
        assert_eq!(
            profile.window_position,
            Some(BrowserViewport {
                width: 0,
                height: 0
            })
        );
        assert!(profile.chromium_sandbox);
        assert!(!profile.devtools);
        assert!(
            !plan
                .args
                .contains(&"--auto-open-devtools-for-tabs".to_owned())
        );
        assert!(
            ![
                "--no-sandbox",
                "--disable-gpu-sandbox",
                "--disable-setuid-sandbox",
                "--no-xshm"
            ]
            .iter()
            .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert_eq!(profile.profile_directory, "Default");
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );
        assert_eq!(profile.user_agent, None);
        assert!(!plan.args.iter().any(|arg| arg.starts_with("--user-agent=")));
        assert!(!profile.disable_security);
        assert!(!profile.deterministic_rendering);
        assert!(
            !CHROME_DISABLE_SECURITY_ARGS
                .iter()
                .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert!(
            !CHROME_DETERMINISTIC_RENDERING_ARGS
                .iter()
                .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert_eq!(profile.browser_start_timeout_ms, 30_000);
        assert_eq!(profile.navigation_timeout_ms, 20_000);
        assert!(!profile.uses_cloud());
        assert_eq!(profile.cloud_create_request(), None);
    }

    #[test]
    fn default_profile_uses_upstream_browser_permissions() {
        let profile = BrowserProfile::default();
        assert_eq!(
            profile.permissions,
            vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()]
        );

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert_eq!(deserialized.permissions, profile.permissions);

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(
            serialized["permissions"],
            json!(["clipboardReadWrite", "notifications"])
        );

        let explicit_empty: BrowserProfile =
            serde_json::from_value(json!({ "permissions": [] })).expect("empty permissions");
        assert!(explicit_empty.permissions.is_empty());
    }

    #[test]
    fn browser_profile_iframe_traversal_defaults_match_upstream() {
        let profile = BrowserProfile::default();
        assert!(profile.cross_origin_iframes);
        assert_eq!(profile.max_iframes, 100);
        assert_eq!(profile.max_iframe_depth, 5);

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert!(deserialized.cross_origin_iframes);
        assert_eq!(deserialized.max_iframes, 100);
        assert_eq!(deserialized.max_iframe_depth, 5);

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(serialized["cross_origin_iframes"], json!(true));
        assert_eq!(serialized["max_iframes"], json!(100));
        assert_eq!(serialized["max_iframe_depth"], json!(5));

        let configured: BrowserProfile = serde_json::from_value(json!({
            "cross_origin_iframes": false,
            "max_iframes": 2,
            "max_iframe_depth": 0
        }))
        .expect("configured profile");
        assert!(!configured.cross_origin_iframes);
        assert_eq!(configured.max_iframes, 2);
        assert_eq!(configured.max_iframe_depth, 0);
    }

    #[test]
    fn browser_profile_paint_order_filtering_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.paint_order_filtering);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["paint_order_filtering"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "paint_order_filtering": false
        }))
        .expect("disabled paint-order filtering profile");
        assert!(!disabled.paint_order_filtering);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["paint_order_filtering"],
            json!(false)
        );
    }

    #[test]
    fn default_profile_uses_upstream_ignore_default_args_shape() {
        let profile = BrowserProfile::default();
        let IgnoreDefaultArgs::List(ignored_args) = &profile.ignore_default_args else {
            panic!("default ignore_default_args should be a list");
        };
        assert_eq!(ignored_args, &default_ignore_default_args());

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert_eq!(
            deserialized.ignore_default_args,
            profile.ignore_default_args
        );

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(serialized["ignore_default_args"], json!(ignored_args));

        let ignored_list: BrowserProfile = serde_json::from_value(json!({
            "ignore_default_args": ["--disable-sync"]
        }))
        .expect("ignore list profile");
        assert_eq!(
            ignored_list.ignore_default_args,
            IgnoreDefaultArgs::List(vec!["--disable-sync".to_owned()])
        );

        let ignored_all: BrowserProfile =
            serde_json::from_value(json!({ "ignore_default_args": true }))
                .expect("ignore all profile");
        assert_eq!(
            ignored_all.ignore_default_args,
            IgnoreDefaultArgs::All(true)
        );
    }

    #[test]
    fn browser_profile_env_defaults_and_coerces_upstream_wire_values() {
        let profile = BrowserProfile::default();
        assert!(profile.env.is_empty());

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert!(deserialized.env.is_empty());

        let profile_with_env: BrowserProfile = serde_json::from_value(json!({
            "env": {
                "BROWSER_USE_HEADLESS": true,
                "BROWSER_USE_SCALE": 2.5,
                "BROWSER_USE_TOKEN": "secret"
            }
        }))
        .expect("deserialize env profile");
        assert_eq!(
            profile_with_env.env,
            BTreeMap::from([
                ("BROWSER_USE_HEADLESS".to_owned(), "true".to_owned()),
                ("BROWSER_USE_SCALE".to_owned(), "2.5".to_owned()),
                ("BROWSER_USE_TOKEN".to_owned(), "secret".to_owned()),
            ])
        );
        assert_eq!(
            serde_json::to_value(&profile_with_env.env).expect("serialize env"),
            json!({
                "BROWSER_USE_HEADLESS": "true",
                "BROWSER_USE_SCALE": "2.5",
                "BROWSER_USE_TOKEN": "secret"
            })
        );
    }

    #[test]
    fn browser_profile_headers_default_omitted_and_round_trip() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.headers, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("headers").is_none());

        let configured: BrowserProfile = serde_json::from_value(json!({
            "headers": {
                "Authorization": "Bearer test-token",
                "X-Browser-Use-Test": "yes"
            }
        }))
        .expect("headers profile");
        assert_eq!(
            configured.headers,
            Some(BTreeMap::from([
                ("Authorization".to_owned(), "Bearer test-token".to_owned()),
                ("X-Browser-Use-Test".to_owned(), "yes".to_owned()),
            ]))
        );
        assert_eq!(
            serde_json::to_value(configured).expect("headers profile json")["headers"],
            json!({
                "Authorization": "Bearer test-token",
                "X-Browser-Use-Test": "yes"
            })
        );
    }

    #[test]
    fn cdp_websocket_request_rejects_invalid_profile_headers() {
        let invalid_name = BTreeMap::from([("Bad Header".to_owned(), "value".to_owned())]);
        let error = cdp_websocket_request("ws://127.0.0.1/devtools/browser/test", &invalid_name)
            .expect_err("invalid header name");
        assert!(
            error
                .to_string()
                .contains("invalid CDP websocket header name")
        );

        let invalid_value = BTreeMap::from([("X-Test".to_owned(), "bad\nvalue".to_owned())]);
        let error = cdp_websocket_request("ws://127.0.0.1/devtools/browser/test", &invalid_value)
            .expect_err("invalid header value");
        assert!(
            error
                .to_string()
                .contains("invalid CDP websocket header value")
        );
    }

    #[test]
    fn browser_permission_grant_params_skip_empty_lists() {
        assert_eq!(browser_permission_grant_params(&[]), None);

        let permissions = vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()];
        assert_eq!(
            browser_permission_grant_params(&permissions),
            Some(json!({
                "permissions": ["clipboardReadWrite", "notifications"]
            }))
        );
    }

    #[test]
    fn permission_grant_failure_lifecycle_event_is_inspectable() {
        let permissions = vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()];
        let event = BrowserLifecycleEvent::permissions_grant_failed(
            &permissions,
            "Browser denied permission grant",
        );

        assert_eq!(event.kind, BrowserLifecycleEventKind::BrowserDiagnostic);
        assert_eq!(event.reason.as_deref(), Some("permissions_grant_failed"));
        assert_eq!(
            event.details.get("permissions").map(String::as_str),
            Some("clipboardReadWrite,notifications")
        );
        assert_eq!(
            event.details.get("permissions_count").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            event.error.as_deref(),
            Some("Browser denied permission grant")
        );
        assert_eq!(
            BrowserLifecycleAdapterEvent::from_lifecycle_event(&event).kind,
            BrowserLifecycleAdapterEventKind::BrowserDiagnostic
        );
    }

    #[tokio::test]
    async fn direct_connect_grants_default_permissions_before_target_attach() {
        let (endpoint, command_log) = cdp_command_test_server(None, 7).await;
        let session = CdpBrowserSession::connect(endpoint)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Browser.grantPermissions",
                "Browser.setDownloadBehavior",
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
        assert_eq!(
            commands[0].params,
            json!({
                "permissions": ["clipboardReadWrite", "notifications"]
            })
        );
        assert_eq!(commands[1].params["behavior"], "allow");
        assert_eq!(commands[1].params["eventsEnabled"], true);
        assert!(
            commands[1].params["downloadPath"]
                .as_str()
                .is_some_and(|path| path.contains("browser-use-downloads-"))
        );
        assert!(
            session
                .downloads_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(session._downloads_dir.is_some());
        assert_eq!(commands[4].session_id.as_deref(), Some("session-1"));
        assert_eq!(commands[5].session_id.as_deref(), Some("session-1"));
        assert_eq!(commands[6].session_id.as_deref(), Some("session-1"));
        assert_eq!(
            commands[6].params,
            json!({
                "width": 1280,
                "height": 720,
                "deviceScaleFactor": 1.0,
                "mobile": false
            })
        );

        let lifecycle_events = session.lifecycle_events().await;
        assert_eq!(
            lifecycle_events
                .iter()
                .filter(|event| event.reason.as_deref() == Some("permissions_grant_failed"))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn direct_connect_skips_empty_permissions_before_target_attach() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
    }

    #[tokio::test]
    async fn direct_connect_keeps_download_behavior_when_auto_pdf_disabled() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            downloads_path: Some(downloads_dir.path().to_path_buf()),
            auto_download_pdfs: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_dir.path().display().to_string(),
                "eventsEnabled": true
            })
        );
        assert!(
            commands
                .iter()
                .any(|command| command.method == "Network.enable")
        );
    }

    #[tokio::test]
    async fn direct_connect_uses_downloads_path_alias_for_download_behavior() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile: BrowserProfile = serde_json::from_value(json!({
            "permissions": [],
            "save_downloads_path": downloads_dir.path().display().to_string()
        }))
        .expect("deserialize alias profile");
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        assert_eq!(
            session.downloads_path.as_deref(),
            Some(downloads_dir.path())
        );

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_dir.path().display().to_string(),
                "eventsEnabled": true
            })
        );
    }

    #[tokio::test]
    async fn direct_connect_generates_session_owned_downloads_path_when_accepted() {
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let downloads_path = session
            .downloads_path
            .clone()
            .expect("generated downloads path");
        assert!(downloads_path.exists());
        assert!(session._downloads_dir.is_some());

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_path.display().to_string(),
                "eventsEnabled": true
            })
        );

        drop(session);
        assert!(!downloads_path.exists());
    }

    #[tokio::test]
    async fn direct_connect_accept_downloads_false_disables_download_path() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            downloads_path: Some(downloads_dir.path().to_path_buf()),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert!(
            !commands
                .iter()
                .any(|command| command.method == "Browser.setDownloadBehavior")
        );
        assert!(session.downloads_path.is_none());
        assert!(session._downloads_dir.is_none());

        session
            .auto_download_pdf_if_needed("https://example.test/report.pdf")
            .await;
        assert!(
            std::fs::read_dir(downloads_dir.path())
                .expect("downloads dir entries")
                .next()
                .is_none()
        );
        assert!(
            session
                .lifecycle_events()
                .await
                .iter()
                .all(|event| event.reason.as_deref() != Some("pdf_auto_download"))
        );
    }

    #[tokio::test]
    async fn direct_connect_sends_profile_headers_in_websocket_handshake() {
        let (endpoint, command_log) = cdp_command_header_test_server(5).await;
        let profile = BrowserProfile {
            headers: Some(BTreeMap::from([
                ("Authorization".to_owned(), "Bearer cdp-token".to_owned()),
                ("X-Browser-Use-Test".to_owned(), "handshake".to_owned()),
            ])),
            permissions: Vec::new(),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect with profile headers");

        let (commands, headers) = command_log.await.expect("cdp command log");
        assert_eq!(
            headers.get("authorization").map(String::as_str),
            Some("Bearer cdp-token")
        );
        assert_eq!(
            headers.get("x-browser-use-test").map(String::as_str),
            Some("handshake")
        );
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
    }

    #[tokio::test]
    async fn direct_connect_applies_configured_viewport_emulation() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            accept_downloads: false,
            viewport: BrowserViewport {
                width: 1024,
                height: 768,
            },
            device_scale_factor: Some(2.5),
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        let command = commands
            .iter()
            .find(|command| command.method == "Emulation.setDeviceMetricsOverride")
            .expect("viewport emulation command");
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            command.params,
            json!({
                "width": 1024,
                "height": 768,
                "deviceScaleFactor": 2.5,
                "mobile": false
            })
        );
    }

    #[tokio::test]
    async fn direct_connect_skips_viewport_emulation_when_no_viewport() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            no_viewport: true,
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert!(
            !commands
                .iter()
                .any(|command| command.method == "Emulation.setDeviceMetricsOverride")
        );
    }

    #[tokio::test]
    async fn direct_connect_records_permission_grant_failures_without_failing() {
        let (endpoint, command_log) =
            cdp_command_test_server(Some("permission grant denied"), 7).await;
        let session = CdpBrowserSession::connect(endpoint)
            .await
            .expect("connect session despite grant failure");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.grantPermissions");
        assert_eq!(commands[1].method, "Browser.setDownloadBehavior");
        assert_eq!(commands[2].method, "Target.getTargets");

        let lifecycle_events = session.lifecycle_events().await;
        let event = lifecycle_events
            .iter()
            .find(|event| event.reason.as_deref() == Some("permissions_grant_failed"))
            .expect("permission grant diagnostic");
        assert_eq!(event.kind, BrowserLifecycleEventKind::BrowserDiagnostic);
        assert!(event.error.as_deref().is_some_and(|error| {
            error.contains("Browser.grantPermissions") && error.contains("permission grant denied")
        }));
    }

    #[test]
    fn browser_profile_security_toggles_default_false_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(!decoded.devtools);
        assert!(!decoded.disable_security);
        assert!(!decoded.deterministic_rendering);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["devtools"], json!(false));
        assert_eq!(encoded["disable_security"], json!(false));
        assert_eq!(encoded["deterministic_rendering"], json!(false));
    }

    #[test]
    fn browser_profile_user_agent_defaults_to_none_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.user_agent, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("user_agent").is_none());
    }

    #[test]
    fn browser_profile_channel_defaults_to_none_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.channel, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("channel").is_none());

        let configured: BrowserProfile =
            serde_json::from_value(json!({ "channel": "chrome-beta" })).expect("channel profile");
        assert_eq!(configured.channel, Some(BrowserChannel::ChromeBeta));
        let configured_json = serde_json::to_value(&configured).expect("configured profile json");
        assert_eq!(configured_json["channel"], json!("chrome-beta"));
    }

    #[test]
    fn browser_profile_profile_directory_defaults_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.profile_directory, "Default");

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["profile_directory"], json!("Default"));
    }

    #[test]
    fn browser_profile_chromium_sandbox_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.chromium_sandbox);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["chromium_sandbox"], json!(true));
    }

    #[test]
    fn browser_profile_devtools_defaults_false_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(!decoded.devtools);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["devtools"], json!(false));
    }

    #[test]
    fn browser_profile_window_geometry_matches_upstream_defaults_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.window_size, None);
        assert_eq!(
            decoded.window_position,
            Some(BrowserViewport {
                width: 0,
                height: 0
            })
        );

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("window_size").is_none());
        assert_eq!(encoded["window_position"], json!({"width": 0, "height": 0}));
    }

    #[test]
    fn browser_profile_viewport_emulation_defaults_and_validation() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.screen, None);
        assert_eq!(decoded.viewport, BrowserViewport::default());
        assert!(!decoded.no_viewport);
        assert_eq!(decoded.device_scale_factor, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("screen").is_none());
        assert_eq!(encoded["viewport"], json!({ "width": 1280, "height": 720 }));
        assert!(encoded.get("no_viewport").is_none());
        assert!(encoded.get("device_scale_factor").is_none());

        let configured: BrowserProfile = serde_json::from_value(json!({
            "screen": { "width": 1920, "height": 1080 },
            "viewport": { "width": 1024, "height": 768 },
            "no_viewport": true,
            "device_scale_factor": 2.5
        }))
        .expect("configured viewport profile");
        assert_eq!(
            configured.screen,
            Some(BrowserViewport {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(
            configured.viewport,
            BrowserViewport {
                width: 1024,
                height: 768
            }
        );
        assert!(configured.no_viewport);
        assert_eq!(configured.device_scale_factor, Some(2.5));

        let negative = serde_json::from_value::<BrowserProfile>(json!({
            "device_scale_factor": -1.0
        }))
        .expect_err("negative device_scale_factor should be rejected");
        assert!(negative.to_string().contains("device_scale_factor"));
    }

    #[test]
    fn browser_profile_keep_alive_preserves_upstream_wire_shape() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.keep_alive, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("keep_alive").is_none());

        let keep_alive: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": true
        }))
        .expect("keep alive profile");
        assert_eq!(keep_alive.keep_alive, Some(true));
        assert!(profile_keeps_launched_browser_alive(&keep_alive));

        let close_on_drop: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": false
        }))
        .expect("close on drop profile");
        assert_eq!(close_on_drop.keep_alive, Some(false));
        assert!(!profile_keeps_launched_browser_alive(&close_on_drop));

        let null_keep_alive: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": null
        }))
        .expect("null keep alive profile");
        assert_eq!(null_keep_alive.keep_alive, None);
        assert!(!profile_keeps_launched_browser_alive(&null_keep_alive));
    }

    #[test]
    fn browser_profile_auto_download_pdfs_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.auto_download_pdfs);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["auto_download_pdfs"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "auto_download_pdfs": false
        }))
        .expect("disabled auto PDF profile");
        assert!(!disabled.auto_download_pdfs);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["auto_download_pdfs"],
            json!(false)
        );
    }

    #[test]
    fn browser_profile_accept_downloads_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.accept_downloads);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["accept_downloads"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "accept_downloads": false,
            "downloads_path": "/tmp/browser-use-rs-disabled-downloads"
        }))
        .expect("disabled downloads profile");
        assert!(!disabled.accept_downloads);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["accept_downloads"],
            json!(false)
        );
    }

    #[test]
    fn browser_profile_downloads_path_aliases_match_upstream() {
        let canonical_path = "/tmp/browser-use-rs-downloads";
        let canonical: BrowserProfile = serde_json::from_value(json!({
            "downloads_path": canonical_path
        }))
        .expect("canonical downloads path profile");
        assert_eq!(
            canonical.downloads_path.as_deref(),
            Some(Path::new(canonical_path))
        );

        let downloads_dir_path = "/tmp/browser-use-rs-downloads-dir";
        let downloads_dir: BrowserProfile = serde_json::from_value(json!({
            "downloads_dir": downloads_dir_path
        }))
        .expect("downloads_dir alias profile");
        assert_eq!(
            downloads_dir.downloads_path.as_deref(),
            Some(Path::new(downloads_dir_path))
        );

        let save_downloads_path = "/tmp/browser-use-rs-save-downloads-path";
        let save_downloads: BrowserProfile = serde_json::from_value(json!({
            "save_downloads_path": save_downloads_path
        }))
        .expect("save_downloads_path alias profile");
        assert_eq!(
            save_downloads.downloads_path.as_deref(),
            Some(Path::new(save_downloads_path))
        );

        let encoded = serde_json::to_value(save_downloads).expect("canonical profile json");
        assert_eq!(encoded["downloads_path"], json!(save_downloads_path));
        assert!(encoded.get("downloads_dir").is_none());
        assert!(encoded.get("save_downloads_path").is_none());
    }

    #[test]
    fn browser_profile_page_load_wait_defaults_and_validation() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.minimum_wait_page_load_time, 0.25);
        assert_eq!(decoded.wait_for_network_idle_page_load_time, 0.5);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["minimum_wait_page_load_time"], json!(0.25));
        assert_eq!(encoded["wait_for_network_idle_page_load_time"], json!(0.5));

        let zero_waits: BrowserProfile = serde_json::from_value(json!({
            "minimum_wait_page_load_time": 0.0,
            "wait_for_network_idle_page_load_time": 0.0
        }))
        .expect("zero wait profile");
        assert_eq!(zero_waits.minimum_wait_page_load_time, 0.0);
        assert_eq!(zero_waits.wait_for_network_idle_page_load_time, 0.0);
        assert!(PageLoadWaitConfig::from_profile(&zero_waits).is_disabled());

        let negative = serde_json::from_value::<BrowserProfile>(json!({
            "minimum_wait_page_load_time": -0.1
        }))
        .expect_err("negative page-load wait should be rejected");
        assert!(negative.to_string().contains("page-load wait"));
    }

    #[test]
    fn network_activity_state_reports_idle_remaining_from_requests_and_finish_events() {
        let start = Instant::now();
        let mut state = NetworkActivityState::new(start);
        let idle_for = Duration::from_millis(500);

        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(200), idle_for),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(500), idle_for),
            None
        );

        state.observe_request_started("request-1", start + Duration::from_millis(600));
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(900), idle_for),
            Some(idle_for)
        );

        state.observe_request_finished("request-1", start + Duration::from_millis(1_000));
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(1_250), idle_for),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(1_500), idle_for),
            None
        );
    }

    #[test]
    fn cloud_browser_request_preserves_proxy_country_tri_state() {
        let omitted = CloudBrowserCreateRequest::default();
        assert_eq!(
            serde_json::to_value(&omitted).expect("request json"),
            json!({})
        );

        let disabled = CloudBrowserCreateRequest {
            proxy_country_code: CloudProxyCountryCode::disabled(),
            ..CloudBrowserCreateRequest::default()
        };
        assert_eq!(
            serde_json::to_value(&disabled).expect("request json"),
            json!({ "proxy_country_code": null })
        );

        let country = CloudBrowserCreateRequest {
            profile_id: Some("profile-123".to_owned()),
            proxy_country_code: CloudProxyCountryCode::country("jp"),
            timeout: Some(60),
            enable_recording: true,
        };
        assert_eq!(
            serde_json::to_value(&country).expect("request json"),
            json!({
                "profile_id": "profile-123",
                "proxy_country_code": "jp",
                "timeout": 60,
                "enable_recording": true
            })
        );
    }

    #[test]
    fn cloud_browser_request_accepts_upstream_aliases() {
        let request: CloudBrowserCreateRequest = serde_json::from_value(json!({
            "cloud_profile_id": "profile-456",
            "cloud_proxy_country_code": null,
            "cloud_timeout": 45,
            "enableRecording": true
        }))
        .expect("alias request");

        assert_eq!(request.profile_id.as_deref(), Some("profile-456"));
        assert_eq!(request.proxy_country_code, CloudProxyCountryCode::Disabled);
        assert_eq!(request.timeout, Some(45));
        assert!(request.enable_recording);
    }

    #[test]
    fn cloud_browser_params_force_cloud_request_without_local_launch_changes() {
        let profile = BrowserProfile {
            cloud_browser_params: Some(CloudBrowserCreateRequest {
                proxy_country_code: CloudProxyCountryCode::disabled(),
                ..CloudBrowserCreateRequest::default()
            }),
            ..BrowserProfile::default()
        };

        assert!(profile.uses_cloud());
        assert_eq!(
            serde_json::to_value(profile.cloud_create_request().expect("cloud request"))
                .expect("request json"),
            json!({ "proxy_country_code": null })
        );

        let plan = profile.launch_plan();
        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
    }

    #[tokio::test]
    async fn cloud_browser_client_tracks_created_session_and_stops_current() {
        let (base_url, server) = cloud_test_server(vec![
            (200, cloud_browser_response_json("browser-123", "running")),
            (200, cloud_browser_response_json("browser-123", "stopped")),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        let created = client
            .create_browser(&CloudBrowserCreateRequest {
                proxy_country_code: CloudProxyCountryCode::disabled(),
                ..CloudBrowserCreateRequest::default()
            })
            .await
            .expect("create cloud browser");
        assert_eq!(created.id, "browser-123");
        assert_eq!(
            client.current_session_id().await.as_deref(),
            Some("browser-123")
        );

        let stopped = client
            .stop_browser(None)
            .await
            .expect("stop current cloud browser");
        assert_eq!(stopped.status, "stopped");
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /api/v2/browsers "));
        assert_eq!(
            request_header(&requests[0], "x-browser-use-api-key"),
            Some("test-key")
        );
        assert_eq!(
            request_body(&requests[0]),
            json!({ "proxy_country_code": null })
        );
        assert!(requests[1].starts_with("PATCH /api/v2/browsers/browser-123 "));
        assert_eq!(request_body(&requests[1]), json!({ "action": "stop" }));
    }

    #[tokio::test]
    async fn cloud_browser_client_sends_extra_headers_on_create_and_stop() {
        let (base_url, server) = cloud_test_server(vec![
            (200, cloud_browser_response_json("browser-extra", "running")),
            (200, cloud_browser_response_json("browser-extra", "stopped")),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        client
            .create_browser_with_headers(
                &CloudBrowserCreateRequest::default(),
                [
                    ("X-Trace-Id", "trace-create"),
                    ("X-Browser-Use-API-Key", "override-key"),
                ],
            )
            .await
            .expect("create cloud browser with extra headers");
        client
            .stop_browser_with_headers(Some("browser-extra"), [("X-Trace-Id", "trace-stop")])
            .await
            .expect("stop cloud browser with extra headers");

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert_eq!(
            request_header(&requests[0], "x-trace-id"),
            Some("trace-create")
        );
        assert_eq!(
            request_header(&requests[0], "x-browser-use-api-key"),
            Some("override-key")
        );
        assert_eq!(
            request_header(&requests[1], "x-trace-id"),
            Some("trace-stop")
        );
        assert_eq!(
            request_header(&requests[1], "x-browser-use-api-key"),
            Some("test-key")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_rejects_invalid_extra_headers_before_request() {
        let error = CloudBrowserClient::with_api_key("test-key")
            .create_browser_with_headers(
                &CloudBrowserCreateRequest::default(),
                [("bad header", "value")],
            )
            .await
            .expect_err("invalid extra header name");
        assert!(
            error
                .to_string()
                .contains("Invalid cloud extra header name")
        );

        let error = CloudBrowserClient::with_api_key("test-key")
            .stop_browser_with_headers(Some("browser-extra"), [("X-Trace-Id", "bad\nvalue")])
            .await
            .expect_err("invalid extra header value");
        assert!(
            error
                .to_string()
                .contains("Invalid cloud extra header value")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_contextualizes_non_success_errors() {
        let (base_url, server) =
            cloud_test_server(vec![(500, json!({ "detail": "create failed" }))]).await;
        let error = CloudBrowserClient::with_api_key("test-key")
            .with_base_url(base_url)
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect_err("create failure should include action context");
        assert!(error.to_string().contains(
            "Failed to create cloud browser: HTTP 500 Internal Server Error - create failed"
        ));
        server.await.expect("create failure server task");

        let (base_url, server) =
            cloud_test_server(vec![(503, json!({ "detail": "stop failed" }))]).await;
        let error = CloudBrowserClient::with_api_key("test-key")
            .with_base_url(base_url)
            .stop_browser(Some("browser-failed"))
            .await
            .expect_err("stop failure should include action context");
        assert!(
            error.to_string().contains(
                "Failed to stop cloud browser: HTTP 503 Service Unavailable - stop failed"
            )
        );
        server.await.expect("stop failure server task");
    }

    #[tokio::test]
    async fn cloud_browser_client_stops_explicit_session_id() {
        let (base_url, server) = cloud_test_server(vec![(
            200,
            cloud_browser_response_json("browser-explicit", "stopped"),
        )])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        let stopped = client
            .stop_browser(Some("browser-explicit"))
            .await
            .expect("stop explicit cloud browser");
        assert_eq!(stopped.id, "browser-explicit");
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("PATCH /api/v2/browsers/browser-explicit "));
        assert_eq!(request_body(&requests[0]), json!({ "action": "stop" }));
    }

    #[tokio::test]
    async fn cloud_browser_client_reports_missing_current_session() {
        let error = CloudBrowserClient::with_api_key("test-key")
            .stop_browser(None)
            .await
            .expect_err("missing current session should fail");

        assert!(error.to_string().contains("No session ID provided"));
    }

    #[tokio::test]
    async fn cloud_browser_client_clears_current_session_on_not_found() {
        let (base_url, server) = cloud_test_server(vec![
            (
                200,
                cloud_browser_response_json("browser-missing", "running"),
            ),
            (404, json!({ "detail": "not found" })),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);
        client
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect("create cloud browser");
        assert_eq!(
            client.current_session_id().await.as_deref(),
            Some("browser-missing")
        );

        let error = client
            .stop_browser(None)
            .await
            .expect_err("404 stop should fail");
        assert!(error.to_string().contains("browser-missing not found"));
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("PATCH /api/v2/browsers/browser-missing "));
    }

    #[test]
    fn cloud_api_key_resolution_prefers_explicit_then_env_then_auth_config() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let auth_config_path = temp_dir.path().join("cloud_auth.json");
        std::fs::write(&auth_config_path, r#"{ "api_token": "config-key" }"#)
            .expect("write cloud auth config");

        assert_eq!(
            resolve_cloud_api_key(
                Some("explicit-key"),
                Some("env-key".to_owned()),
                Some(&auth_config_path)
            )
            .as_deref(),
            Some("explicit-key")
        );
        assert_eq!(
            resolve_cloud_api_key(
                Some("  "),
                Some("env-key".to_owned()),
                Some(&auth_config_path)
            )
            .as_deref(),
            Some("env-key")
        );
        assert_eq!(
            resolve_cloud_api_key(None, Some("  ".to_owned()), Some(&auth_config_path)).as_deref(),
            Some("config-key")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_uses_auth_config_api_token() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let auth_config_path = temp_dir.path().join("cloud_auth.json");
        std::fs::write(&auth_config_path, r#"{ "api_token": "config-key" }"#)
            .expect("write cloud auth config");
        let (base_url, server) = cloud_test_server(vec![(
            200,
            cloud_browser_response_json("browser-config-token", "running"),
        )])
        .await;
        let client = CloudBrowserClient::new()
            .with_auth_config_path(auth_config_path)
            .with_base_url(base_url);

        let created = client
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect("create cloud browser");
        assert_eq!(created.id, "browser-config-token");

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("x-browser-use-api-key: config-key")
        );
    }

    #[test]
    fn cloud_auth_config_fallback_ignores_missing_empty_and_corrupt_files() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let missing_path = temp_dir.path().join("missing.json");
        assert_eq!(load_cloud_auth_api_token(Some(&missing_path)), None);

        let corrupt_path = temp_dir.path().join("corrupt.json");
        std::fs::write(&corrupt_path, "{").expect("write corrupt config");
        assert_eq!(load_cloud_auth_api_token(Some(&corrupt_path)), None);

        let empty_path = temp_dir.path().join("empty.json");
        std::fs::write(&empty_path, r#"{ "api_token": "  " }"#).expect("write empty config");
        assert_eq!(load_cloud_auth_api_token(Some(&empty_path)), None);
    }

    #[test]
    fn cloud_auth_config_path_matches_upstream_env_layout() {
        assert_eq!(
            cloud_auth_config_path(
                Some(PathBuf::from("~/browser-use")),
                Some(PathBuf::from("/xdg")),
                Some(PathBuf::from("/home/alice"))
            ),
            PathBuf::from("/home/alice/browser-use/cloud_auth.json")
        );
        assert_eq!(
            cloud_auth_config_path(None, Some(PathBuf::from("/xdg")), None),
            PathBuf::from("/xdg/browseruse/cloud_auth.json")
        );
        assert_eq!(
            cloud_auth_config_path(None, None, Some(PathBuf::from("/home/alice"))),
            PathBuf::from("/home/alice/.config/browseruse/cloud_auth.json")
        );
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
                bypass: None,
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
                .contains(&"--profile-directory=Default".to_owned())
        );
        assert!(
            plan.args
                .contains(&"--proxy-server=http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn launch_plan_preserves_env_without_changing_args() {
        let profile = BrowserProfile {
            env: BTreeMap::from([
                ("BROWSER_USE_HEADLESS".to_owned(), "false".to_owned()),
                ("BROWSER_USE_TOKEN".to_owned(), "secret".to_owned()),
            ]),
            args: vec!["--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert_eq!(plan.env, profile.env);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
        assert!(plan.args.iter().any(|arg| arg == "--headless=new"));
    }

    #[test]
    fn launch_plan_emits_representative_upstream_default_args() {
        let profile = BrowserProfile::default();
        let plan = profile.launch_plan();

        for arg in [
            "--disable-background-networking",
            "--disable-popup-blocking",
            "--disable-sync",
            "--enable-features=NetworkService,NetworkServiceInProcess",
        ] {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg == arg),
                "missing upstream default arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_suppresses_listed_default_args() {
        let profile = BrowserProfile {
            ignore_default_args: IgnoreDefaultArgs::List(vec![
                "--disable-sync".to_owned(),
                "--disable-popup-blocking".to_owned(),
            ]),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(!plan.args.iter().any(|arg| arg == "--disable-sync"));
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--disable-popup-blocking")
        );
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--disable-background-networking")
        );
    }

    #[test]
    fn launch_plan_suppresses_all_default_args_when_requested() {
        let profile = BrowserProfile {
            ignore_default_args: IgnoreDefaultArgs::All(true),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        for arg in [
            "--disable-background-networking",
            "--disable-popup-blocking",
            "--disable-sync",
            "--no-first-run",
            "--no-default-browser-check",
        ] {
            assert!(
                !plan.args.iter().any(|plan_arg| plan_arg == arg),
                "default arg {arg} should be suppressed"
            );
        }
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert!(plan.args.iter().any(|arg| arg == "--window-size=1280,720"));
        assert!(plan.args.iter().any(|arg| arg == "--headless=new"));
    }

    #[test]
    fn launch_plan_merges_disable_features_values_in_order() {
        let profile = BrowserProfile {
            disable_security: true,
            args: vec![
                "--disable-features=MediaRouter,Translate,IsolateOrigins".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();
        let disable_features_args = plan
            .args
            .iter()
            .filter(|arg| arg.starts_with("--disable-features="))
            .map(String::as_str)
            .collect::<Vec<_>>();

        assert_eq!(
            disable_features_args,
            vec!["--disable-features=IsolateOrigins,site-per-process,MediaRouter,Translate"]
        );
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_dedupes_duplicate_switches_with_last_value() {
        let profile = BrowserProfile {
            user_agent: Some("GeneratedAgent/1.0".to_owned()),
            args: vec![
                "--user-agent=CallerAgent/2.0".to_owned(),
                "--remote-debugging-port=9333".to_owned(),
            ],
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--user-agent=GeneratedAgent/1.0")
        );
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--user-agent=CallerAgent/2.0")
        );
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert_eq!(
            plan.args.last(),
            Some(&"--remote-debugging-port=9333".to_owned())
        );
    }

    #[test]
    fn launch_plan_emits_default_profile_directory_with_user_data_dir() {
        let profile = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let user_data_index = arg_index(&plan.args, "--user-data-dir=/tmp/browser-use-rs-profile");
        let profile_directory_index = arg_index(&plan.args, "--profile-directory=Default");

        assert_eq!(profile_directory_index, user_data_index + 1);
    }

    #[test]
    fn launch_plan_omits_empty_or_orphan_profile_directory() {
        let no_user_data_dir = BrowserProfile {
            profile_directory: "Profile 2".to_owned(),
            ..BrowserProfile::default()
        }
        .launch_plan();
        assert!(
            !no_user_data_dir
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );

        let empty_profile_directory = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            profile_directory: String::new(),
            ..BrowserProfile::default()
        }
        .launch_plan();
        assert!(
            !empty_profile_directory
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );
    }

    #[test]
    fn launch_plan_places_custom_profile_directory_before_generated_and_custom_args() {
        let profile = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            profile_directory: "Profile 2".to_owned(),
            disable_security: true,
            args: vec![
                "--profile-directory=Override".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let user_data_index = arg_index(&plan.args, "--user-data-dir=/tmp/browser-use-rs-profile");
        let security_index = arg_index(&plan.args, "--disable-site-isolation-trials");
        let custom_profile_directory_index = arg_index(&plan.args, "--profile-directory=Override");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--profile-directory=Profile 2")
        );
        assert!(user_data_index < security_index);
        assert!(security_index < custom_profile_directory_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_chromium_sandbox_args_when_disabled() {
        let profile = BrowserProfile {
            chromium_sandbox: false,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DOCKER_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing chromium_sandbox=false launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_keeps_chromium_sandbox_args_before_custom_args() {
        let profile = BrowserProfile {
            chromium_sandbox: false,
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            args: vec!["--no-sandbox=false".to_owned(), "--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let profile_directory_index = arg_index(&plan.args, "--profile-directory=Default");
        let first_custom_arg_index = arg_index(&plan.args, "--no-sandbox=false");

        assert!(!plan.args.iter().any(|arg| arg == "--no-sandbox"));
        for arg in CHROME_DOCKER_ARGS {
            if *arg == "--no-sandbox" {
                continue;
            }
            assert!(
                arg_index(&plan.args, arg) < first_custom_arg_index,
                "generated chromium_sandbox=false launch arg {arg} should come before caller args"
            );
        }
        assert!(profile_directory_index < first_custom_arg_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_devtools_arg_when_headful() {
        let profile = BrowserProfile {
            headless: false,
            devtools: true,
            ..BrowserProfile::default()
        };

        let plan = profile.try_launch_plan().expect("devtools launch plan");

        assert!(
            plan.args
                .contains(&"--auto-open-devtools-for-tabs".to_owned())
        );
        assert!(!plan.args.contains(&"--headless=new".to_owned()));
    }

    #[test]
    fn launch_plan_rejects_devtools_with_headless() {
        let profile = BrowserProfile {
            headless: true,
            devtools: true,
            ..BrowserProfile::default()
        };

        let error = profile
            .try_launch_plan()
            .expect_err("headless devtools should fail launch planning");

        assert!(
            error
                .to_string()
                .contains("headless=True and devtools=True cannot both be set")
        );
    }

    #[test]
    fn launch_plan_rejects_no_viewport_with_headless() {
        let profile = BrowserProfile {
            headless: true,
            no_viewport: true,
            ..BrowserProfile::default()
        };

        let error = profile
            .try_launch_plan()
            .expect_err("headless no_viewport should fail launch planning");

        assert!(
            error
                .to_string()
                .contains("headless=True and no_viewport=True cannot both be set")
        );
    }

    #[test]
    fn launch_plan_keeps_devtools_arg_before_custom_args() {
        let profile = BrowserProfile {
            headless: false,
            devtools: true,
            args: vec![
                "--auto-open-devtools-for-tabs=false".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.try_launch_plan().expect("devtools launch plan");
        let custom_devtools_index = arg_index(&plan.args, "--auto-open-devtools-for-tabs=false");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--auto-open-devtools-for-tabs")
        );
        assert!(custom_devtools_index < arg_index(&plan.args, "--custom-last"));
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_uses_explicit_window_size_without_mutating_viewport() {
        let profile = BrowserProfile {
            window_size: Some(BrowserViewport {
                width: 1920,
                height: 1400,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert_eq!(profile.viewport, BrowserViewport::default());
        assert!(plan.args.contains(&"--window-size=1920,1400".to_owned()));
        assert!(!plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn launch_plan_can_use_screen_as_window_size_fallback() {
        let profile = BrowserProfile {
            screen: Some(BrowserViewport {
                width: 1440,
                height: 900,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--window-size=1440,900".to_owned()));
        assert!(!plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn viewport_emulation_params_match_cdp_shape() {
        let params = viewport_emulation_params(ViewportEmulationConfig {
            viewport: Some(BrowserViewport {
                width: 1024,
                height: 768,
            }),
            device_scale_factor: 2.0,
        })
        .expect("viewport params");

        assert_eq!(
            params,
            json!({
                "width": 1024,
                "height": 768,
                "deviceScaleFactor": 2.0,
                "mobile": false
            })
        );

        assert_eq!(
            viewport_emulation_params(ViewportEmulationConfig {
                viewport: None,
                device_scale_factor: 2.0,
            }),
            None
        );
    }

    #[test]
    fn launch_plan_emits_window_position() {
        let profile = BrowserProfile {
            window_position: Some(BrowserViewport {
                width: 40,
                height: 80,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--window-position=40,80".to_owned()));
    }

    #[test]
    fn launch_plan_keeps_window_geometry_before_custom_args() {
        let profile = BrowserProfile {
            window_size: Some(BrowserViewport {
                width: 1440,
                height: 900,
            }),
            window_position: Some(BrowserViewport {
                width: 10,
                height: 20,
            }),
            args: vec![
                "--window-size=1,1".to_owned(),
                "--window-position=2,2".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let custom_size_index = arg_index(&plan.args, "--window-size=1,1");
        let custom_position_index = arg_index(&plan.args, "--window-position=2,2");

        assert!(!plan.args.iter().any(|arg| arg == "--window-size=1440,900"));
        assert!(!plan.args.iter().any(|arg| arg == "--window-position=10,20"));
        assert!(custom_size_index < custom_position_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_disable_security_args() {
        let profile = BrowserProfile {
            disable_security: true,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DISABLE_SECURITY_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing disable_security launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_emits_deterministic_rendering_args() {
        let profile = BrowserProfile {
            deterministic_rendering: true,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DETERMINISTIC_RENDERING_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing deterministic_rendering launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_keeps_security_and_rendering_args_before_custom_args() {
        let profile = BrowserProfile {
            disable_security: true,
            deterministic_rendering: true,
            args: vec![
                "--force-device-scale-factor=1".to_owned(),
                "--custom-last".to_owned(),
            ],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                bypass: Some("localhost".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let first_custom_arg_index = arg_index(&plan.args, "--force-device-scale-factor=1");

        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
        for arg in CHROME_DISABLE_SECURITY_ARGS
            .iter()
            .chain(CHROME_DETERMINISTIC_RENDERING_ARGS.iter())
        {
            if *arg == "--force-device-scale-factor=2" {
                continue;
            }
            assert!(
                arg_index(&plan.args, arg) < first_custom_arg_index,
                "generated launch arg {arg} should come before caller args"
            );
        }
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--force-device-scale-factor=2")
        );
        assert!(
            arg_index(&plan.args, "--disable-site-isolation-trials")
                < arg_index(&plan.args, "--deterministic-mode")
        );
        assert!(arg_index(&plan.args, "--proxy-bypass-list=localhost") < first_custom_arg_index);
    }

    #[test]
    fn launch_plan_omits_empty_user_agent() {
        let profile = BrowserProfile {
            user_agent: Some(String::new()),
            args: vec!["--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(!plan.args.iter().any(|arg| arg.starts_with("--user-agent=")));
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_user_agent_before_custom_args() {
        let profile = BrowserProfile {
            user_agent: Some("BrowserUseRust/0.4".to_owned()),
            args: vec![
                "--user-agent=OverrideAgent/1.0".to_owned(),
                "--custom-last".to_owned(),
            ],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                bypass: None,
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let proxy_index = arg_index(&plan.args, "--proxy-server=http://127.0.0.1:8080");
        let custom_user_agent_index = arg_index(&plan.args, "--user-agent=OverrideAgent/1.0");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--user-agent=BrowserUseRust/0.4")
        );
        assert!(proxy_index < custom_user_agent_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn proxy_settings_serializes_optional_bypass() {
        let without_bypass = ProxySettings {
            server: "socks5://127.0.0.1:1080".to_owned(),
            bypass: None,
            username: None,
            password: None,
        };
        assert_eq!(
            serde_json::to_value(&without_bypass).expect("serialize proxy without bypass"),
            json!({ "server": "socks5://127.0.0.1:1080" })
        );

        let with_bypass = ProxySettings {
            server: "http://proxy.internal:8080".to_owned(),
            bypass: Some("localhost,127.0.0.1,*.internal".to_owned()),
            username: Some("alice".to_owned()),
            password: None,
        };
        assert_eq!(
            serde_json::to_value(&with_bypass).expect("serialize proxy with bypass"),
            json!({
                "server": "http://proxy.internal:8080",
                "bypass": "localhost,127.0.0.1,*.internal",
                "username": "alice"
            })
        );

        let decoded: ProxySettings = serde_json::from_value(json!({
            "server": "http://proxy.internal:8080"
        }))
        .expect("deserialize proxy without bypass");
        assert_eq!(decoded.bypass, None);
    }

    #[test]
    fn launch_plan_emits_proxy_bypass_after_proxy_server() {
        let profile = BrowserProfile {
            args: vec!["--disable-gpu".to_owned()],
            proxy: Some(ProxySettings {
                server: "http://proxy.internal:8080".to_owned(),
                bypass: Some("localhost,127.0.0.1,*.internal".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let proxy_server_index = plan
            .args
            .iter()
            .position(|arg| arg == "--proxy-server=http://proxy.internal:8080")
            .expect("proxy server arg");
        let proxy_bypass_index = plan
            .args
            .iter()
            .position(|arg| arg == "--proxy-bypass-list=localhost,127.0.0.1,*.internal")
            .expect("proxy bypass arg");

        assert_eq!(proxy_bypass_index, proxy_server_index + 1);
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn launch_plan_skips_proxy_bypass_without_server_or_value() {
        let bypass_without_server = BrowserProfile {
            proxy: Some(ProxySettings {
                server: String::new(),
                bypass: Some("localhost".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        }
        .launch_plan();

        assert!(
            !bypass_without_server
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-server="))
        );
        assert!(
            !bypass_without_server
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-bypass-list="))
        );

        let empty_bypass = BrowserProfile {
            proxy: Some(ProxySettings {
                server: "http://proxy.internal:8080".to_owned(),
                bypass: Some(String::new()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        }
        .launch_plan();

        assert!(
            empty_bypass
                .args
                .contains(&"--proxy-server=http://proxy.internal:8080".to_owned())
        );
        assert!(
            !empty_bypass
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-bypass-list="))
        );
    }

    #[test]
    fn iframe_targets_match_parent_and_frame_urls() {
        let targets = json!({
            "targetInfos": [
                {
                    "type": "iframe",
                    "targetId": "child",
                    "parentId": "root",
                    "url": "http://127.0.0.1:8081/child#section"
                },
                {
                    "type": "iframe",
                    "targetId": "unrelated",
                    "parentId": "other-page",
                    "url": "http://127.0.0.1:8081/child"
                },
                {
                    "type": "iframe",
                    "targetId": "fallback",
                    "url": "https://example.test/frame"
                },
                {
                    "type": "page",
                    "targetId": "page",
                    "url": "https://example.test/frame"
                }
            ]
        });
        let frame_infos = vec![
            FrameElementInfo {
                url: "http://127.0.0.1:8081/child".to_owned(),
                offset: FrameOffset { x: 12, y: 34 },
            },
            FrameElementInfo {
                url: "https://example.test/frame".to_owned(),
                offset: FrameOffset { x: 56, y: 78 },
            },
        ];

        let infos = iframe_target_infos_from_targets(
            &targets,
            "root",
            &frame_infos,
            IframeTraversalConfig::from_profile(&BrowserProfile::default()),
        );

        assert_eq!(
            infos,
            vec![
                IframeTargetInfo {
                    target_id: "child".to_owned(),
                    offset: FrameOffset { x: 12, y: 34 },
                    depth: 1,
                },
                IframeTargetInfo {
                    target_id: "fallback".to_owned(),
                    offset: FrameOffset { x: 56, y: 78 },
                    depth: 1,
                },
            ]
        );
    }

    #[test]
    fn iframe_target_limits_honor_profile_controls() {
        let targets = json!({
            "targetInfos": [
                {
                    "type": "iframe",
                    "targetId": "one",
                    "parentId": "root",
                    "url": "https://example.test/one"
                },
                {
                    "type": "iframe",
                    "targetId": "two",
                    "parentId": "root",
                    "url": "https://example.test/two"
                },
                {
                    "type": "iframe",
                    "targetId": "three",
                    "parentId": "root",
                    "url": "https://example.test/three"
                }
            ]
        });
        let frame_infos = vec![
            FrameElementInfo {
                url: "https://example.test/one".to_owned(),
                offset: FrameOffset { x: 1, y: 1 },
            },
            FrameElementInfo {
                url: "https://example.test/two".to_owned(),
                offset: FrameOffset { x: 2, y: 2 },
            },
            FrameElementInfo {
                url: "https://example.test/three".to_owned(),
                offset: FrameOffset { x: 3, y: 3 },
            },
        ];
        let limited = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 2,
            max_iframe_depth: 5,
        };

        let infos = iframe_target_infos_from_targets(&targets, "root", &frame_infos, limited);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].target_id, "one");
        assert_eq!(infos[1].target_id, "two");

        let disabled = IframeTraversalConfig {
            cross_origin_iframes: false,
            max_iframes: 100,
            max_iframe_depth: 5,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, disabled).is_empty()
        );

        let zero_depth = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 100,
            max_iframe_depth: 0,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, zero_depth).is_empty()
        );

        let zero_iframes = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 0,
            max_iframe_depth: 5,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, zero_iframes)
                .is_empty()
        );
    }

    #[test]
    fn interactive_snapshot_script_carries_iframe_traversal_limits() {
        let script = interactive_elements_js(
            IframeTraversalConfig {
                cross_origin_iframes: true,
                max_iframes: 7,
                max_iframe_depth: 2,
            },
            true,
        );

        assert!(script.contains("const maxIframeDepth = 2;"));
        assert!(script.contains("const maxIframeDocuments = 7;"));
        assert!(script.contains("if (depth >= maxIframeDepth) return;"));
        assert!(script.contains("if (visitedIframeDocuments >= maxIframeDocuments) return;"));
        assert!(script.contains("visitChildren(frameDocument, { x: offset.x + rect.x, y: offset.y + rect.y }, depth + 1);"));
    }

    #[test]
    fn cached_iframe_fallback_uses_target_local_index() {
        let state = SerializedDomState::from_elements(vec![
            test_dom_bound_element(1, "root-target", "Root iframe", None),
            test_dom_bound_element(2, "child-target", "Child button", None),
            test_dom_bound_element(3, "child-target", "Child input", None),
        ]);
        let current_page = AttachedPage {
            target_id: "root-target".to_owned(),
            session_id: "root-session".to_owned(),
        };
        let cached = CachedDomElementRef {
            element: state.selector_map[&3].clone(),
            target_local_index: target_local_index_for_global_index(
                &state.selector_map,
                3,
                "child-target",
            ),
        };

        assert_eq!(cached.target_local_index, 2);
        assert_eq!(
            index_fallback_target_id(&current_page, Some(&cached)),
            "child-target"
        );
        assert_eq!(
            target_local_index_for_global_index(&state.selector_map, 1, "root-target"),
            1
        );
    }

    #[test]
    fn merged_dom_states_renumber_elements_and_preserve_targets() {
        let root = SerializedDomState::from_elements(vec![test_dom_bound_element(
            8,
            "root-target",
            "Root button",
            None,
        )])
        .with_page_stats(DomPageStats {
            interactive_elements: 1,
            total_elements: 3,
            ..DomPageStats::default()
        })
        .with_eval_root(DomEvalNode::element("html").with_children(vec![
            DomEvalNode::element("body").with_children(vec![
                DomEvalNode::element("iframe").with_attribute("title", "Child frame"),
            ]),
        ]));
        let mut child = SerializedDomState::from_elements(vec![test_dom_bound_element(
            1,
            "child-target",
            "Child input",
            Some(ElementBounds {
                x: 5,
                y: 7,
                width: 90,
                height: 20,
            }),
        )])
        .with_page_stats(DomPageStats {
            interactive_elements: 1,
            total_elements: 2,
            ..DomPageStats::default()
        })
        .with_eval_root(DomEvalNode::element("html").with_children(vec![
            DomEvalNode::element("body").with_children(vec![
                DomEvalNode::element("button")
                    .with_children(vec![DomEvalNode::text("Child input")])
                    .interactive(88),
            ]),
        ]));
        offset_dom_state_bounds(&mut child, FrameOffset { x: 100, y: 40 });

        let merged = merge_dom_states(root, vec![child]);

        assert_eq!(merged.element_count(), 2);
        assert_eq!(merged.selector_map[&1].target_id, "root-target");
        assert_eq!(merged.selector_map[&2].target_id, "child-target");
        assert_eq!(merged.selector_map[&2].name.as_deref(), Some("Child input"));
        assert_eq!(
            merged.selector_map[&2].bounds,
            Some(ElementBounds {
                x: 105,
                y: 47,
                width: 90,
                height: 20,
            })
        );
        assert_eq!(merged.page_stats.interactive_elements, 2);
        assert_eq!(merged.page_stats.total_elements, 5);
        let eval = merged.eval_representation();
        assert!(
            eval.contains("#iframe-content"),
            "merged eval tree missed iframe content: {eval}"
        );
        assert!(
            eval.contains("[i_88] <button>Child input"),
            "merged eval tree missed child backend marker: {eval}"
        );
    }

    fn test_dom_bound_element(
        index: u32,
        target_id: &str,
        name: &str,
        bounds: Option<ElementBounds>,
    ) -> DomElementRef {
        DomElementRef {
            index,
            target_id: target_id.to_owned(),
            backend_node_id: index.into(),
            node_id: Some(index.into()),
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some(name.to_owned()),
            text: Some(name.to_owned()),
            attributes: BTreeMap::new(),
            bounds,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    async fn spawn_static_html_server(
        body: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind static html server");
        let addr = listener.local_addr().expect("static html server address");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 1024];
                    let _ = stream.read(&mut buffer).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        (addr, server)
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
    fn url_policy_watchdog_closes_disallowed_new_target_events() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec!["evil.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let event = CdpEvent {
            method: "Target.targetCreated".to_owned(),
            params: json!({
                "targetInfo": {
                    "type": "page",
                    "targetId": "popup-target",
                    "url": "https://evil.test/popup"
                }
            }),
            session_id: None,
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &event),
            Some(UrlPolicyWatchdogAction::CloseTarget {
                target_id: "popup-target".to_owned(),
                url: "https://evil.test/popup".to_owned(),
                reason: "in_prohibited_domains".to_owned(),
            })
        );
    }

    #[test]
    fn url_policy_watchdog_ignores_empty_target_urls() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec!["evil.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let target_event = CdpEvent {
            method: "Target.targetCreated".to_owned(),
            params: json!({
                "targetInfo": {
                    "type": "page",
                    "targetId": "popup-target",
                    "url": ""
                }
            }),
            session_id: None,
        };
        let frame_event = CdpEvent {
            method: "Page.frameNavigated".to_owned(),
            params: json!({ "frame": { "id": "frame-1", "url": "" } }),
            session_id: Some("current-session".to_owned()),
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &target_event),
            None
        );
        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &frame_event),
            None
        );
    }

    #[test]
    fn url_policy_watchdog_resets_current_page_navigation_events() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["safe.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let event = CdpEvent {
            method: "Page.frameNavigated".to_owned(),
            params: json!({
                "frame": {
                    "id": "frame-1",
                    "url": "https://blocked.test/redirect"
                }
            }),
            session_id: Some("current-session".to_owned()),
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &event),
            Some(UrlPolicyWatchdogAction::ResetCurrent {
                session_id: "current-session".to_owned(),
                url: "https://blocked.test/redirect".to_owned(),
                reason: "not_in_allowed_domains".to_owned(),
            })
        );
    }

    #[test]
    fn browser_security_events_format_state_diagnostics() {
        let mut events = VecDeque::new();
        push_security_event(
            &mut events,
            BrowserSecurityEvent::prevented_navigation(
                "https://blocked.test/direct".to_owned(),
                "not_in_allowed_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::closed_popup(
                "https://evil.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::reset_current(
                "https://blocked.test/redirect".to_owned(),
                "not_in_allowed_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "CDP target is already detached".to_owned(),
            ),
        );

        let (recent_events, closed_popup_messages, browser_errors) =
            security_event_state_fields(&events);
        let recent_events = recent_events.expect("recent security events");

        assert!(recent_events.contains("no browser navigation was started"));
        assert!(recent_events.contains("Closed popup https://evil.test/popup"));
        assert!(recent_events.contains("reset current tab to about:blank"));
        assert!(recent_events.contains("Failed to close popup https://stuck.test/popup"));
        assert_eq!(
            closed_popup_messages,
            vec!["Closed popup https://evil.test/popup (in_prohibited_domains)"]
        );
        assert_eq!(
            browser_errors,
            vec![
                "Failed to close popup https://stuck.test/popup (in_prohibited_domains): CDP target is already detached"
            ]
        );
        assert_eq!(
            events[0].lifecycle_event.kind,
            BrowserLifecycleEventKind::NavigationBlocked
        );
        assert_eq!(
            events[1].lifecycle_event.kind,
            BrowserLifecycleEventKind::PopupClosed
        );
        assert_eq!(
            events[2].lifecycle_event.kind,
            BrowserLifecycleEventKind::CurrentTargetReset
        );
        assert_eq!(
            events[3].lifecycle_event.kind,
            BrowserLifecycleEventKind::PopupCloseFailed
        );
    }

    #[test]
    fn browser_security_events_are_bounded() {
        let mut events = VecDeque::new();
        for index in 0..(MAX_SECURITY_EVENTS + 2) {
            push_security_event(
                &mut events,
                BrowserSecurityEvent::closed_popup(
                    format!("https://blocked-{index}.test/popup"),
                    "in_prohibited_domains".to_owned(),
                ),
            );
        }

        let (recent_events, closed_popup_messages, browser_errors) =
            security_event_state_fields(&events);
        let recent_events = recent_events.expect("recent security events");

        assert_eq!(events.len(), MAX_SECURITY_EVENTS);
        assert_eq!(closed_popup_messages.len(), MAX_SECURITY_EVENTS);
        assert!(browser_errors.is_empty());
        assert!(!recent_events.contains("blocked-0.test"));
        assert!(!recent_events.contains("blocked-1.test"));
        assert!(recent_events.contains("blocked-2.test"));
        assert!(recent_events.contains("blocked-9.test"));
    }

    #[test]
    fn browser_lifecycle_events_cover_target_and_navigation_transitions() {
        let events = vec![
            BrowserLifecycleEvent::browser_connected("http://127.0.0.1:9222"),
            BrowserLifecycleEvent::target_created("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_switched("target-1"),
            BrowserLifecycleEvent::navigation_started("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_completed("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_closed("target-1"),
            BrowserSecurityEvent::reset_current(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "No target with given id found".to_owned(),
            )
            .lifecycle_event,
        ];

        assert_eq!(
            events.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::BrowserConnected,
                &BrowserLifecycleEventKind::TargetCreated,
                &BrowserLifecycleEventKind::TargetSwitched,
                &BrowserLifecycleEventKind::NavigationStarted,
                &BrowserLifecycleEventKind::NavigationCompleted,
                &BrowserLifecycleEventKind::TargetClosed,
                &BrowserLifecycleEventKind::CurrentTargetReset,
                &BrowserLifecycleEventKind::PopupCloseFailed,
            ]
        );
        assert_eq!(events[1].target_id.as_deref(), Some("target-1"));
        assert_eq!(events[3].url.as_deref(), Some("https://example.test"));
        assert_eq!(events[6].reason.as_deref(), Some("not_in_allowed_domains"));
        assert_eq!(
            events[7].error.as_deref(),
            Some("No target with given id found")
        );

        let json = serde_json::to_value(&events).expect("serialize lifecycle events");
        assert_eq!(json[0]["kind"], "browser_connected");
        assert_eq!(json[4]["kind"], "navigation_completed");
        assert!(json[4].get("details").is_none());
    }

    #[test]
    fn browser_lifecycle_events_cover_remaining_upstream_shapes() {
        let events = vec![
            BrowserLifecycleEvent::browser_reconnecting("http://127.0.0.1:9222", 2, 3),
            BrowserLifecycleEvent::browser_reconnected("http://127.0.0.1:9222", 2, "1.25"),
            BrowserLifecycleEvent::target_crashed("target-1", "Inspector target crashed"),
            BrowserLifecycleEvent::navigation_failed(
                "target-1",
                "https://example.test/slow",
                "net::ERR_FAILED",
            ),
            BrowserLifecycleEvent::network_timeout("target-1", "https://example.test/slow", "8"),
            BrowserLifecycleEvent::javascript_dialog_handled(
                "https://example.test",
                "confirm",
                "Continue?",
                true,
            ),
            BrowserLifecycleEvent::download_started(
                "download-guid",
                "https://example.test/report.pdf",
                "report.pdf",
            ),
            BrowserLifecycleEvent::download_progress(
                "download-guid",
                1024,
                Some(4096),
                "inProgress",
            ),
            BrowserLifecycleEvent::file_downloaded(
                "download-guid",
                "/tmp/report.pdf",
                "report.pdf",
                4096,
            ),
            BrowserLifecycleEvent::storage_state_saved("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::storage_state_loaded("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::browser_stopped("graceful_stop"),
        ];

        assert_eq!(
            events.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::BrowserReconnecting,
                &BrowserLifecycleEventKind::BrowserReconnected,
                &BrowserLifecycleEventKind::TargetCrashed,
                &BrowserLifecycleEventKind::NavigationFailed,
                &BrowserLifecycleEventKind::NetworkTimeout,
                &BrowserLifecycleEventKind::JavaScriptDialogHandled,
                &BrowserLifecycleEventKind::DownloadStarted,
                &BrowserLifecycleEventKind::DownloadProgress,
                &BrowserLifecycleEventKind::FileDownloaded,
                &BrowserLifecycleEventKind::StorageStateSaved,
                &BrowserLifecycleEventKind::StorageStateLoaded,
                &BrowserLifecycleEventKind::BrowserStopped,
            ]
        );

        assert_eq!(events[0].details["attempt"], "2");
        assert_eq!(events[1].details["downtime_seconds"], "1.25");
        assert_eq!(events[5].details["dialog_message"], "Continue?".to_owned());
        assert_eq!(events[7].details["total_bytes"], "4096");
        assert_eq!(events[9].details["cookies_count"], "4");

        let json = serde_json::to_value(&events).expect("serialize lifecycle events");
        assert_eq!(json[2]["kind"], "target_crashed");
        assert_eq!(json[5]["details"]["action"], "accepted");
        assert_eq!(json[8]["details"]["file_name"], "report.pdf");
    }

    #[test]
    fn browser_lifecycle_adapter_events_map_upstream_taxonomy() {
        let events = vec![
            BrowserLifecycleEvent::browser_close_requested(),
            BrowserLifecycleEvent::browser_connected("http://127.0.0.1:9222"),
            BrowserLifecycleEvent::browser_stopped("graceful_stop"),
            BrowserLifecycleEvent::browser_reconnecting("http://127.0.0.1:9222", 1, 3),
            BrowserLifecycleEvent::browser_reconnected("http://127.0.0.1:9222", 1, "0.250"),
            BrowserLifecycleEvent::target_created("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_closed("target-1"),
            BrowserLifecycleEvent::target_switched("target-1"),
            BrowserLifecycleEvent::target_crashed("target-1", "Inspector target crashed"),
            BrowserLifecycleEvent::navigation_started("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_completed("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_failed(
                "target-1",
                "https://example.test",
                "net::ERR_FAILED",
            ),
            BrowserLifecycleEvent::network_timeout("target-1", "https://example.test", "8"),
            BrowserSecurityEvent::prevented_navigation(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::reset_current(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::reset_current_failed(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
                "reset failed".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::closed_popup(
                "https://blocked.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "No target with given id found".to_owned(),
            )
            .lifecycle_event,
            BrowserLifecycleEvent::javascript_dialog_handled(
                "https://example.test",
                "alert",
                "Hello",
                false,
            ),
            BrowserLifecycleEvent::download_started(
                "download-guid",
                "https://example.test/report.pdf",
                "report.pdf",
            ),
            BrowserLifecycleEvent::download_progress(
                "download-guid",
                1024,
                Some(4096),
                "inProgress",
            ),
            BrowserLifecycleEvent::file_downloaded(
                "download-guid",
                "/tmp/report.pdf",
                "report.pdf",
                4096,
            ),
            BrowserLifecycleEvent::storage_state_saved("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::storage_state_loaded("/tmp/storage.json", 4, 2),
        ];

        let adapter_events = browser_lifecycle_adapter_events(&events);

        assert_eq!(
            adapter_events
                .iter()
                .map(|event| &event.kind)
                .collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleAdapterEventKind::BrowserStop,
                &BrowserLifecycleAdapterEventKind::BrowserConnected,
                &BrowserLifecycleAdapterEventKind::BrowserStopped,
                &BrowserLifecycleAdapterEventKind::BrowserReconnecting,
                &BrowserLifecycleAdapterEventKind::BrowserReconnected,
                &BrowserLifecycleAdapterEventKind::TabCreated,
                &BrowserLifecycleAdapterEventKind::TabClosed,
                &BrowserLifecycleAdapterEventKind::AgentFocusChanged,
                &BrowserLifecycleAdapterEventKind::TargetCrashed,
                &BrowserLifecycleAdapterEventKind::NavigationStarted,
                &BrowserLifecycleAdapterEventKind::NavigationComplete,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserDiagnostic,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserDiagnostic,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::JavaScriptDialogHandled,
                &BrowserLifecycleAdapterEventKind::DownloadStarted,
                &BrowserLifecycleAdapterEventKind::DownloadProgress,
                &BrowserLifecycleAdapterEventKind::FileDownloaded,
                &BrowserLifecycleAdapterEventKind::StorageState,
                &BrowserLifecycleAdapterEventKind::StorageState,
            ]
        );
        assert_eq!(
            adapter_events[7].source_kind,
            BrowserLifecycleEventKind::TargetSwitched
        );
        assert_eq!(adapter_events[7].target_id.as_deref(), Some("target-1"));
        assert_eq!(
            adapter_events[14].source_kind,
            BrowserLifecycleEventKind::CurrentTargetReset
        );

        let json = serde_json::to_value(&adapter_events).expect("serialize adapter events");
        assert_eq!(json[7]["kind"], "agent_focus_changed");
        assert_eq!(json[10]["kind"], "navigation_complete");
        assert_eq!(json[10]["source_kind"], "navigation_completed");
    }

    #[tokio::test]
    async fn lifecycle_adapter_subscription_maps_facade_events() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let subscription = BrowserLifecycleEventSubscription::new(event_rx);
        let mut adapter_subscription = BrowserLifecycleAdapterEventSubscription::new(subscription);

        assert_eq!(adapter_subscription.try_recv().expect("empty stream"), None);

        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-1"))
            .expect("send lifecycle event");

        let event = adapter_subscription
            .recv()
            .await
            .expect("adapter lifecycle event");
        assert_eq!(
            event.kind,
            BrowserLifecycleAdapterEventKind::AgentFocusChanged
        );
        assert_eq!(event.source_kind, BrowserLifecycleEventKind::TargetSwitched);
        assert_eq!(event.target_id.as_deref(), Some("target-1"));
    }

    #[test]
    fn lifecycle_watchdog_maps_cdp_crash_and_download_events() {
        let mut active_requests = HashMap::new();
        track_network_request(
            &mut active_requests,
            &CdpEvent {
                method: "Network.requestWillBeSent".to_owned(),
                params: json!({
                    "requestId": "request-1",
                    "request": {
                        "url": "https://example.test/api/report",
                        "method": "POST"
                    },
                    "type": "Fetch",
                }),
                session_id: Some("session-1".to_owned()),
            },
        );
        let started_at = active_requests
            .get_mut("request-1")
            .expect("tracked request")
            .started_at;
        active_requests
            .get_mut("request-1")
            .expect("tracked request")
            .started_at = started_at - Duration::from_secs(11);
        let timeout_events = lifecycle_events_for_timed_out_network_requests(
            &mut active_requests,
            started_at,
            Duration::from_secs(10),
        );
        assert_eq!(timeout_events.len(), 1);
        assert_eq!(
            timeout_events[0].kind,
            BrowserLifecycleEventKind::NetworkTimeout
        );
        assert_eq!(timeout_events[0].details["request_id"], "request-1");
        assert!(active_requests.is_empty());

        let websocket_closed = lifecycle_event_for_websocket_closed(&CdpEvent {
            method: "browser-use-rs.websocket-closed".to_owned(),
            params: json!({
                "reason": "websocket_error",
                "error": "connection reset",
            }),
            session_id: None,
        });
        assert_eq!(
            websocket_closed.kind,
            BrowserLifecycleEventKind::BrowserStopped
        );
        assert_eq!(websocket_closed.reason.as_deref(), Some("websocket_error"));
        assert_eq!(websocket_closed.error.as_deref(), Some("connection reset"));
        assert!(should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "websocket_stream_ended" }),
                session_id: None,
            },
            false,
            false,
        ));
        assert!(!should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "connection_actor_stopped" }),
                session_id: None,
            },
            false,
            false,
        ));
        assert!(!should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "websocket_error" }),
                session_id: None,
            },
            true,
            false,
        ));
        assert_eq!(
            cdp_reconnect_delay_for_attempt(4),
            Duration::from_millis(4_000)
        );

        let reconnecting = lifecycle_event_for_websocket_reconnecting(
            &cdp_websocket_reconnecting_event("http://127.0.0.1:9222", 2, 3),
        )
        .expect("reconnecting lifecycle event");
        assert_eq!(
            reconnecting.kind,
            BrowserLifecycleEventKind::BrowserReconnecting
        );
        assert_eq!(reconnecting.details["attempt"], "2");
        assert_eq!(reconnecting.details["max_attempts"], "3");

        let reconnected =
            lifecycle_event_for_websocket_reconnected(&cdp_websocket_reconnected_event(
                "http://127.0.0.1:9222",
                2,
                Duration::from_millis(1_250),
                4,
            ))
            .expect("reconnected lifecycle event");
        assert_eq!(
            reconnected.kind,
            BrowserLifecycleEventKind::BrowserReconnected
        );
        assert_eq!(reconnected.details["downtime_seconds"], "1.250");
        assert_eq!(reconnected.details["connection_generation"], "4");

        let reconnect_failed =
            lifecycle_event_for_websocket_reconnect_failed(&cdp_websocket_reconnect_failed_event(
                "http://127.0.0.1:9222",
                3,
                Duration::from_millis(7_000),
                Some("connection refused".to_owned()),
            ));
        assert_eq!(
            reconnect_failed.kind,
            BrowserLifecycleEventKind::BrowserStopped
        );
        assert_eq!(reconnect_failed.reason.as_deref(), Some("reconnect_failed"));
        assert_eq!(
            reconnect_failed.error.as_deref(),
            Some("connection refused")
        );

        let crash_event = CdpEvent {
            method: "Target.targetCrashed".to_owned(),
            params: json!({
                "targetId": "target-1",
                "status": "crashed",
                "errorCode": 139,
            }),
            session_id: Some("session-1".to_owned()),
        };
        let crash_events = lifecycle_events_for_target_crash(&crash_event);
        assert_eq!(crash_events.len(), 1);
        assert_eq!(
            crash_events[0].kind,
            BrowserLifecycleEventKind::TargetCrashed
        );
        assert_eq!(crash_events[0].target_id.as_deref(), Some("target-1"));
        assert_eq!(crash_events[0].error.as_deref(), Some("crashed (139)"));
        assert_eq!(crash_events[0].details["session_id"], "session-1");

        let download_start = lifecycle_event_for_download_start(&CdpEvent {
            method: "Browser.downloadWillBegin".to_owned(),
            params: json!({
                "guid": "download-guid",
                "url": "https://example.test/report.pdf",
                "suggestedFilename": "report.pdf",
            }),
            session_id: None,
        })
        .expect("download start event");
        assert_eq!(
            download_start.kind,
            BrowserLifecycleEventKind::DownloadStarted
        );
        assert_eq!(download_start.details["suggested_filename"], "report.pdf");

        let sanitized_download_start = lifecycle_event_for_download_start(&CdpEvent {
            method: "Browser.downloadWillBegin".to_owned(),
            params: json!({
                "guid": "download-guid",
                "url": "https://example.test/report.pdf",
                "suggestedFilename": "../../etc/passwd",
            }),
            session_id: None,
        })
        .expect("sanitized download start event");
        assert_eq!(
            sanitized_download_start.details["suggested_filename"],
            "passwd"
        );

        let download_progress = lifecycle_events_for_download_progress(&CdpEvent {
            method: "Browser.downloadProgress".to_owned(),
            params: json!({
                "guid": "download-guid",
                "receivedBytes": 4096,
                "totalBytes": 4096,
                "state": "completed",
                "filePath": "/tmp/report.pdf",
            }),
            session_id: None,
        });
        assert_eq!(
            download_progress
                .iter()
                .map(|event| &event.kind)
                .collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::DownloadProgress,
                &BrowserLifecycleEventKind::FileDownloaded,
            ]
        );
        assert_eq!(download_progress[1].details["file_name"], "report.pdf");

        let sanitized_download_progress = lifecycle_events_for_download_progress(&CdpEvent {
            method: "Browser.downloadProgress".to_owned(),
            params: json!({
                "guid": "download-guid",
                "receivedBytes": 4096,
                "totalBytes": 4096,
                "state": "completed",
                "filePath": "/tmp/../../escape.bin",
            }),
            session_id: None,
        });
        assert_eq!(
            sanitized_download_progress[1].details["file_name"],
            "escape.bin"
        );
    }

    #[test]
    fn download_filename_sanitization_matches_upstream_security_boundary() {
        assert_eq!(sanitize_download_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_download_filename("/etc/shadow"), "shadow");
        assert_eq!(
            sanitize_download_filename("..\\..\\Windows\\System32\\config.txt"),
            "config.txt"
        );
        assert_eq!(sanitize_download_filename("a/b\\c/../d.pdf"), "d.pdf");
        for malicious in ["..", ".", "/", "\\", "../", "..\\", "/.", "\\.", "/.."] {
            assert_eq!(
                sanitize_download_filename(malicious),
                "download",
                "{malicious:?} should fall back to default"
            );
        }
        assert_eq!(sanitize_download_filename("file.txt\0.exe"), "file.txt.exe");
        assert_eq!(sanitize_download_filename(""), "download");
        assert_eq!(sanitize_download_filename("report.pdf"), "report.pdf");
        assert_eq!(
            sanitize_download_filename("file with spaces.pdf"),
            "file with spaces.pdf"
        );
        assert_eq!(sanitize_download_filename(".bashrc"), ".bashrc");
        assert_eq!(sanitize_download_filename("résumé.pdf"), "résumé.pdf");
        assert_eq!(sanitize_download_filename("文档.pdf"), "文档.pdf");
    }

    #[test]
    fn pdf_viewer_url_detection_is_conservative() {
        assert!(is_pdf_viewer_url("https://example.test/report.pdf"));
        assert!(is_pdf_viewer_url(
            "https://example.test/report.PDF?download=1#page=2"
        ));
        assert!(is_pdf_viewer_url("https://example.test/viewer/pdf/123"));
        assert!(!is_pdf_viewer_url("https://example.test/report.html"));
        assert!(!is_pdf_viewer_url(
            "https://example.test/report.html?file=report.pdf"
        ));
    }

    #[test]
    fn pdf_download_filename_uses_safe_pdf_basename() {
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/docs/report.pdf?x=1"),
            "report.pdf"
        );
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/pdf/monthly%20report"),
            "monthly report.pdf"
        );
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/docs/..%2Fsecret.pdf"),
            "secret.pdf"
        );
    }

    #[test]
    fn cdp_auto_pdf_candidate_uses_response_metadata() {
        let event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/download?id=123",
                    "mimeType": "application/pdf",
                    "headers": {
                        "Content-Disposition": "attachment; filename*=UTF-8''report%20final",
                        "Content-Type": "application/pdf; charset=binary"
                    }
                }
            }),
            session_id: Some("session-1".to_owned()),
        };

        let candidate = cdp_auto_pdf_candidate_from_response(&event).expect("pdf candidate");
        assert_eq!(candidate.request_id, "request-1");
        assert_eq!(candidate.request_key, "session-1:request-1");
        assert_eq!(candidate.session_id.as_deref(), Some("session-1"));
        assert_eq!(candidate.url, "https://example.test/download?id=123");
        assert_eq!(candidate.file_name, "report final.pdf");
    }

    #[test]
    fn cdp_auto_pdf_candidate_ignores_non_pdf_responses() {
        let event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/index.html?file=report.pdf",
                    "mimeType": "text/html",
                    "headers": {
                        "Content-Type": "text/html"
                    }
                }
            }),
            session_id: None,
        };

        assert!(cdp_auto_pdf_candidate_from_response(&event).is_none());
    }

    #[tokio::test]
    async fn cdp_auto_pdf_state_deduplicates_url_cache_and_paths() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let downloaded_urls = Arc::new(Mutex::new(BTreeMap::new()));
        let state = Arc::new(CdpAutoPdfDownloadState {
            downloads_path: temp_dir.path().to_path_buf(),
            downloaded_urls,
            candidates: Mutex::new(BTreeMap::new()),
        });
        let response_event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/report.pdf",
                    "mimeType": "application/pdf",
                    "headers": {}
                }
            }),
            session_id: Some("session-1".to_owned()),
        };
        let finish_event = CdpEvent {
            method: "Network.loadingFinished".to_owned(),
            params: json!({ "requestId": "request-1" }),
            session_id: Some("session-1".to_owned()),
        };

        state.observe_response(&response_event).await;
        let candidate = state
            .take_finished_candidate(&finish_event)
            .await
            .expect("first candidate");
        let event = state
            .write_candidate(&candidate, b"%PDF-1.7")
            .await
            .expect("write pdf");
        assert_eq!(event.details["file_name"], "report.pdf");
        let first_path = temp_dir.path().join("report.pdf");
        assert!(first_path.exists());

        state.observe_response(&response_event).await;
        assert!(state.take_finished_candidate(&finish_event).await.is_none());

        std::fs::remove_file(&first_path).expect("remove cached pdf");
        state.observe_response(&response_event).await;
        let second_candidate = state
            .take_finished_candidate(&finish_event)
            .await
            .expect("stale cache redownload candidate");
        std::fs::write(&first_path, b"existing").expect("seed duplicate filename");
        let second = state
            .write_candidate(&second_candidate, b"%PDF-1.7")
            .await
            .expect("write deduped pdf");
        assert_eq!(second.details["file_name"], "report-1.pdf");
    }

    #[tokio::test]
    async fn cdp_auto_pdf_lifecycle_downloads_response_body() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 1).await;
        let connection = CdpConnection::connect(&endpoint)
            .await
            .expect("connect cdp");
        let downloaded_urls = Arc::new(Mutex::new(BTreeMap::new()));
        let state = Arc::new(CdpAutoPdfDownloadState {
            downloads_path: temp_dir.path().to_path_buf(),
            downloaded_urls,
            candidates: Mutex::new(BTreeMap::new()),
        });
        let response_event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/download",
                    "mimeType": "application/octet-stream",
                    "headers": {
                        "Content-Disposition": "attachment; filename=cdp-report.pdf",
                        "Content-Type": "application/pdf"
                    }
                }
            }),
            session_id: Some("session-1".to_owned()),
        };
        let finish_event = CdpEvent {
            method: "Network.loadingFinished".to_owned(),
            params: json!({ "requestId": "request-1" }),
            session_id: Some("session-1".to_owned()),
        };

        state.observe_response(&response_event).await;
        let auto_pdf_download = Some(state);
        let event = cdp_auto_pdf_lifecycle_event(&connection, &auto_pdf_download, &finish_event)
            .await
            .expect("auto PDF lifecycle event");
        assert_eq!(event.kind, BrowserLifecycleEventKind::FileDownloaded);
        assert_eq!(event.reason.as_deref(), Some("pdf_auto_download"));
        assert_eq!(event.details["file_name"], "cdp-report.pdf");
        assert_eq!(event.details["file_size"], "17");
        assert_eq!(
            tokio::fs::read(temp_dir.path().join("cdp-report.pdf"))
                .await
                .expect("downloaded pdf bytes"),
            b"%PDF-1.7 cdp body"
        );

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Network.getResponseBody");
        assert_eq!(commands[0].params, json!({ "requestId": "request-1" }));
        assert_eq!(commands[0].session_id.as_deref(), Some("session-1"));
    }

    #[tokio::test]
    async fn auto_pdf_download_writes_once_and_reuses_session_cache() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let (url, hits) = pdf_download_test_server(b"%PDF-1.4 test").await;
        let session = test_session_for_pdf_downloads(Some(temp_dir.path().to_path_buf()), true);

        let event = session
            .auto_download_pdf(&url, temp_dir.path())
            .await
            .expect("download PDF")
            .expect("first download event");
        assert_eq!(event.kind, BrowserLifecycleEventKind::FileDownloaded);
        assert_eq!(event.reason.as_deref(), Some("pdf_auto_download"));
        assert_eq!(event.details["auto_download"], "true");
        assert_eq!(event.details["file_name"], "report.pdf");
        assert_eq!(hits.await.expect("PDF server hits"), 1);
        let downloaded_path = temp_dir.path().join("report.pdf");
        assert_eq!(
            tokio::fs::read(&downloaded_path)
                .await
                .expect("downloaded PDF bytes"),
            b"%PDF-1.4 test"
        );

        let duplicate = session
            .auto_download_pdf(&url, temp_dir.path())
            .await
            .expect("duplicate cache lookup");
        assert!(duplicate.is_none());
    }

    #[tokio::test]
    async fn disabled_auto_pdf_download_does_not_touch_downloads_path() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let session = test_session_for_pdf_downloads(Some(temp_dir.path().to_path_buf()), false);

        session
            .auto_download_pdf_if_needed("https://example.test/report.pdf")
            .await;

        assert!(
            std::fs::read_dir(temp_dir.path())
                .expect("downloads dir entries")
                .next()
                .is_none()
        );
        assert!(session.lifecycle_events().await.is_empty());
    }

    #[tokio::test]
    async fn unique_download_path_avoids_existing_files_inside_download_dir() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let existing = temp_dir.path().join("report.pdf");
        tokio::fs::write(&existing, b"existing")
            .await
            .expect("write existing PDF");

        let next = unique_download_path(temp_dir.path(), "../../report.pdf")
            .await
            .expect("unique path");
        tokio::fs::write(&next, b"new")
            .await
            .expect("write unique PDF");

        assert_eq!(next, temp_dir.path().join("report-1.pdf"));
        assert!(is_path_contained(&next, temp_dir.path()));
    }

    #[test]
    fn path_containment_rejects_directory_escape() {
        let temp_dir = TempDir::new().expect("temp downloads dir");
        let downloads_dir = temp_dir.path();
        let nested_dir = downloads_dir.join("nested");
        std::fs::create_dir(&nested_dir).expect("nested downloads dir");
        let nested_file = nested_dir.join("report.pdf");
        std::fs::write(&nested_file, b"pdf").expect("nested file");

        assert!(is_path_contained(downloads_dir, downloads_dir));
        assert!(is_path_contained(&nested_file, downloads_dir));
        assert!(!is_path_contained(
            &downloads_dir.join("../escape.bin"),
            downloads_dir
        ));

        let sibling_dir = downloads_dir
            .parent()
            .expect("downloads dir parent")
            .join(format!(
                "{}_sibling",
                downloads_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("downloads dir name")
            ));
        std::fs::create_dir(&sibling_dir).expect("sibling dir");
        let sibling_file = sibling_dir.join("report.pdf");
        std::fs::write(&sibling_file, b"pdf").expect("sibling file");
        assert!(!is_path_contained(&sibling_file, downloads_dir));
    }

    #[tokio::test]
    async fn cdp_connection_rejects_stale_registered_sessions() {
        let (request_tx, _request_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(1);
        let connection = CdpConnection {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            session_generations: Arc::new(Mutex::new(HashMap::new())),
        };

        connection.register_attached_session("session-1").await;
        connection
            .ensure_session_generation_current(Some("session-1"))
            .await
            .expect("session is current before reconnect");
        connection
            .connection_generation
            .fetch_add(1, Ordering::Relaxed);

        let error = connection
            .ensure_session_generation_current(Some("session-1"))
            .await
            .expect_err("session is stale after reconnect generation advances");
        assert!(matches!(error, BrowserError::Transport(_)));
        assert!(error.to_string().contains("stale after reconnect"));

        connection
            .ensure_session_generation_current(Some("unknown-session"))
            .await
            .expect("unknown sessions are left to Chrome");
    }

    #[test]
    fn storage_state_counts_browser_use_shape() {
        let storage_state = json!({
                "cookies": [
                    { "name": "sid", "value": "1", "domain": ".example.test", "path": "/" },
                    { "name": "pref", "value": "dark", "domain": ".example.test", "path": "/" }
                ],
                "origins": [
                    {
                        "origin": "https://example.test",
                        "localStorage": [{ "name": "theme", "value": "dark" }],
                        "sessionStorage": [{ "name": "tab", "value": "reports" }]
                    }
                ]
        });
        assert_eq!(storage_state_counts(&storage_state), (2, 1));
        assert_eq!(storage_state_counts(&json!({})), (0, 0));

        let script = origin_storage_apply_script(&storage_state["origins"][0])
            .expect("origin storage apply script");
        assert!(script.contains(r#"const expectedOrigin = "https://example.test";"#));
        assert!(script.contains(r#""theme":"dark""#));
        assert!(script.contains(r#""tab":"reports""#));
        assert!(
            origin_storage_apply_script(&json!({
                "origin": "https://empty.test",
                "localStorage": [],
                "sessionStorage": []
            }))
            .is_none()
        );

        let frame_tree = json!({
            "frameTree": {
                "frame": {
                    "id": "root",
                    "url": "https://example.test/dashboard",
                    "securityOrigin": "https://example.test"
                },
                "childFrames": [
                    {
                        "frame": {
                            "id": "child-1",
                            "url": "https://accounts.example.test/login"
                        }
                    },
                    {
                        "frame": {
                            "id": "child-2",
                            "url": "about:blank",
                            "securityOrigin": "null"
                        }
                    }
                ]
            }
        });
        assert_eq!(
            frame_security_origins_from_result(&frame_tree)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                "https://accounts.example.test".to_owned(),
                "https://example.test".to_owned()
            ]
        );

        let dom_storage_items = dom_storage_entries_to_items(Some(&json!([
            ["zeta", "last"],
            ["alpha", "first"],
            ["ignored"],
        ])));
        assert_eq!(
            dom_storage_items,
            vec![
                json!({ "name": "alpha", "value": "first" }),
                json!({ "name": "zeta", "value": "last" }),
            ]
        );

        let mut origin_states = BTreeMap::new();
        upsert_origin_storage_state(
            &mut origin_states,
            json!({
                "origin": "https://example.test",
                "localStorage": [{ "name": "theme", "value": "dark" }],
                "sessionStorage": []
            }),
        );
        upsert_origin_storage_state(
            &mut origin_states,
            json!({
                "origin": "https://example.test",
                "localStorage": [{ "name": "theme", "value": "light" }],
                "sessionStorage": [{ "name": "tab", "value": "reports" }]
            }),
        );
        assert_eq!(
            origin_states["https://example.test"]["localStorage"],
            json!([{ "name": "theme", "value": "light" }])
        );
        assert_eq!(
            origin_states["https://example.test"]["sessionStorage"],
            json!([{ "name": "tab", "value": "reports" }])
        );
    }

    #[test]
    fn browser_lifecycle_events_are_bounded() {
        let mut events = VecDeque::new();
        for index in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            push_lifecycle_event(
                &mut events,
                BrowserLifecycleEvent::navigation_completed(
                    format!("target-{index}"),
                    format!("https://example.test/{index}"),
                ),
            );
        }

        assert_eq!(events.len(), MAX_LIFECYCLE_EVENTS);
        assert_eq!(events[0].target_id.as_deref(), Some("target-2"));
        assert_eq!(
            events.back().and_then(|event| event.target_id.as_deref()),
            Some("target-33")
        );
    }

    #[test]
    fn lifecycle_event_bus_publishes_while_history_stays_bounded() {
        let (event_tx, mut event_rx) = broadcast::channel(64);
        let mut events = VecDeque::new();

        for index in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            push_lifecycle_event_and_publish(
                &mut events,
                &event_tx,
                BrowserLifecycleEvent::navigation_completed(
                    format!("target-{index}"),
                    format!("https://example.test/{index}"),
                ),
            );
        }

        let mut received_targets = Vec::new();
        for _ in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            let event = event_rx.try_recv().expect("published lifecycle event");
            received_targets.push(event.target_id.expect("target id"));
        }

        assert_eq!(events.len(), MAX_LIFECYCLE_EVENTS);
        assert_eq!(events[0].target_id.as_deref(), Some("target-2"));
        assert_eq!(
            events.back().and_then(|event| event.target_id.as_deref()),
            Some("target-33")
        );
        assert_eq!(
            received_targets.first().map(String::as_str),
            Some("target-0")
        );
        assert_eq!(
            received_targets.last().map(String::as_str),
            Some("target-33")
        );
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_hides_broadcast_empty_and_closed_states() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        assert_eq!(subscription.try_recv().expect("empty stream"), None);

        let event = BrowserLifecycleEvent::target_switched("target-1");
        event_tx.send(event.clone()).expect("event sent");
        assert_eq!(
            subscription.try_recv().expect("published event"),
            Some(event)
        );

        drop(event_tx);
        assert!(matches!(
            subscription.recv().await,
            Err(BrowserLifecycleEventStreamError::Closed)
        ));
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_reports_lagged_consumers() {
        let (event_tx, event_rx) = broadcast::channel(1);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-1"))
            .expect("first event sent");
        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-2"))
            .expect("second event sent");

        assert!(matches!(
            subscription.try_recv(),
            Err(BrowserLifecycleEventStreamError::Lagged(_))
        ));
        assert_eq!(
            subscription.try_recv().expect("latest event"),
            Some(BrowserLifecycleEvent::target_switched("target-2"))
        );
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_resubscribes_at_current_tail() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        let first_event = BrowserLifecycleEvent::target_switched("target-1");
        event_tx
            .send(first_event.clone())
            .expect("first event sent");
        assert_eq!(subscription.recv().await.expect("first event"), first_event);

        let mut resubscribed = subscription.resubscribe();
        let second_event = BrowserLifecycleEvent::target_switched("target-2");
        event_tx
            .send(second_event.clone())
            .expect("second event sent");

        assert_eq!(
            resubscribed.recv().await.expect("resubscribed event"),
            second_event
        );
    }

    #[tokio::test]
    async fn closed_lifecycle_event_subscription_is_immediately_closed() {
        let mut subscription = BrowserLifecycleEventSubscription::closed();

        assert!(matches!(
            subscription.recv().await,
            Err(BrowserLifecycleEventStreamError::Closed)
        ));
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
    fn url_policy_blocks_non_standard_ipv4_forms_when_configured() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });

        for url in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0x7F.0x0.0x0.0x1/",
            "http://0177.0.0.1/",
            "http://127.1/",
            "http://127.0.1/",
            "http://10.1/",
        ] {
            assert!(
                !policy.is_allowed(url),
                "non-standard IPv4 should be blocked: {url}"
            );
        }

        assert!(policy.is_allowed("http://127.0.0.1.evil.test/"));
        assert!(policy.is_allowed("http://2130706433.evil.test/"));
        assert!(!is_ip_address("999.999.999.999"));
        assert!(!is_ip_address("1.2.3.4.5"));
    }

    #[test]
    fn ip_classifier_canonicalizes_encoded_and_unicode_hosts() {
        for host in [
            "%30x7f000001",
            "%31%32%37.0.0.1",
            "%32%31%33%30%37%30%36%34%33%33",
            "１２７.０.０.１",
            "０x7f000001",
            "①②⑦.⓪.⓪.①",
            "127。0。0。1",
            "127｡0｡0｡1",
            "127．0．0．1",
            "①②⑦。⓪。⓪。①",
        ] {
            assert!(
                is_ip_address(host),
                "host should classify as an IP address: {host}"
            );
        }

        for host in [
            "%",
            "%zz",
            "%2",
            "café.example",
            "xn--caf-dma.example",
            "日本.example",
            "xn--wgv71a.example",
            "2130706433.evil.test",
        ] {
            assert!(
                !is_ip_address(host),
                "host should remain classified as a domain: {host}"
            );
        }
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
    fn cloud_browser_response_converts_to_devtools_endpoint() {
        let response = CloudBrowserResponse {
            id: "browser-123".to_owned(),
            status: "running".to_owned(),
            live_url: "https://cloud.browser-use.com/live/browser-123".to_owned(),
            cdp_url: "wss://cdp.browser-use.com/devtools/browser/abc123".to_owned(),
            timeout_at: "2026-05-18T20:00:00Z".to_owned(),
            started_at: "2026-05-18T19:00:00Z".to_owned(),
            finished_at: None,
        };

        let endpoint = response.devtools_endpoint().expect("devtools endpoint");

        assert_eq!(endpoint.http_url, "https://cdp.browser-use.com");
        assert_eq!(
            endpoint.websocket_url,
            "wss://cdp.browser-use.com/devtools/browser/abc123"
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
    fn interactive_snapshot_summarizes_compound_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("inputCompoundComponents"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("compoundComponentsFor"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("audio[controls]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("video[controls]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Browse Files"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Files Selected"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Color Picker"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Toggle Disclosure"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Fullscreen"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("audio[controls]"));
        assert!(action_script.contains("video[controls]"));
    }

    #[test]
    fn interactive_snapshot_preserves_automation_attributes() {
        for attribute in [
            "aria-controls",
            "aria-disabled",
            "aria-haspopup",
            "aria-keyshortcuts",
            "aria-level",
            "aria-live",
            "aria-multiselectable",
            "aria-owns",
            "aria-placeholder",
            "aria-readonly",
            "aria-required",
            "aria-valuemax",
            "aria-valuetext",
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
        assert!(INTERACTIVE_ELEMENTS_JS.contains("booleanAttributeNames"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("booleanAttributeNames.has(name)"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs[name] = 'true'"));
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
                    "value": { "type": "string", "value": "Ready" },
                    "description": { "type": "computedString", "value": "Primary action" },
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
        assert_eq!(
            button.properties.get("value").map(String::as_str),
            Some("Ready")
        );
        assert_eq!(
            button.properties.get("description").map(String::as_str),
            Some("Primary action")
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
                properties: BTreeMap::from([
                    ("description".to_owned(), "Primary action".to_owned()),
                    ("expanded".to_owned(), "true".to_owned()),
                ]),
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
        assert_eq!(
            element.attributes.get("ax_name").map(String::as_str),
            Some("Save settings")
        );
        assert_eq!(
            element.attributes.get("ax_description").map(String::as_str),
            Some("Primary action")
        );
    }

    #[test]
    fn dom_state_parser_applies_ax_hidden_disabled_veto_and_preserves_metadata() {
        let accessibility = BTreeMap::from([
            (
                "browser-use-rs-hidden".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 41,
                    node_id: Some(81),
                    role: Some("button".to_owned()),
                    name: Some("Hidden action".to_owned()),
                    properties: BTreeMap::from([
                        ("focusable".to_owned(), "true".to_owned()),
                        ("hidden".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
            (
                "browser-use-rs-disabled".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 42,
                    node_id: Some(82),
                    role: Some("button".to_owned()),
                    name: Some("Disabled action".to_owned()),
                    properties: BTreeMap::from([
                        ("disabled".to_owned(), "true".to_owned()),
                        ("focusable".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
            (
                "browser-use-rs-editable".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 43,
                    node_id: Some(83),
                    role: Some("textbox".to_owned()),
                    name: Some("Search".to_owned()),
                    properties: BTreeMap::from([
                        ("autocomplete".to_owned(), "list".to_owned()),
                        ("editable".to_owned(), "true".to_owned()),
                        ("focusable".to_owned(), "true".to_owned()),
                        ("settable".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
        ]);
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [
                    {
                        "index": 1,
                        "tag_name": "button",
                        "attributes": { "id": "hidden-action" },
                        "ax_ref": "browser-use-rs-hidden",
                        "is_visible": true,
                        "is_interactive": true
                    },
                    {
                        "index": 2,
                        "tag_name": "button",
                        "attributes": { "id": "disabled-action" },
                        "ax_ref": "browser-use-rs-disabled",
                        "is_visible": true,
                        "is_interactive": true
                    },
                    {
                        "index": 3,
                        "tag_name": "input",
                        "attributes": { "id": "search", "type": "text" },
                        "ax_ref": "browser-use-rs-editable",
                        "is_visible": true,
                        "is_interactive": true
                    }
                ]
            }),
            &accessibility,
        )
        .expect("dom state");

        assert_eq!(state.selector_map.len(), 1);
        assert!(!state.selector_map.contains_key(&1));
        assert!(!state.selector_map.contains_key(&2));

        let editable = state.selector_map.get(&3).expect("editable element");
        assert_eq!(editable.backend_node_id, 43);
        assert_eq!(editable.role.as_deref(), Some("textbox"));
        assert_eq!(editable.name.as_deref(), Some("Search"));
        assert_eq!(
            editable.attributes.get("focusable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("editable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("settable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("autocomplete").map(String::as_str),
            Some("list")
        );
        assert_eq!(
            editable.attributes.get("ax_name").map(String::as_str),
            Some("Search")
        );
        assert_eq!(
            state.llm_representation(),
            "[3] <input type=text id=search autocomplete=list> Search"
        );
        assert_eq!(
            state.llm_representation_with_attributes(&[
                "focusable".to_owned(),
                "editable".to_owned(),
                "settable".to_owned()
            ]),
            "[3] <input focusable=true editable=true settable=true> Search"
        );
    }

    #[test]
    fn dom_state_parser_preserves_native_boolean_attributes() {
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [{
                    "index": 1,
                    "tag_name": "input",
                    "name": "Invoice id",
                    "text": "INV-123",
                    "attributes": {
                        "id": "invoice",
                        "readonly": "true",
                        "required": "true",
                        "multiple": "true"
                    },
                    "is_visible": true,
                    "is_interactive": true
                }]
            }),
            &BTreeMap::new(),
        )
        .expect("dom state");

        let llm = state.llm_representation();
        assert!(
            llm.contains("[1] <input id=invoice multiple=true required=true> Invoice id INV-123"),
            "DOM state did not render default native boolean attributes: {llm}"
        );
        assert_eq!(
            state.selector_map[&1]
                .attributes
                .get("readonly")
                .map(String::as_str),
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
    fn dom_state_parser_carries_eval_tree() {
        let accessibility = BTreeMap::from([(
            "browser-use-rs-1".to_owned(),
            AccessibilityNodeInfo {
                backend_node_id: 55,
                node_id: Some(77),
                role: Some("button".to_owned()),
                name: Some("Save settings".to_owned()),
                properties: BTreeMap::from([(
                    "description".to_owned(),
                    "Persists account settings".to_owned(),
                )]),
            },
        )]);
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [{
                    "index": 1,
                    "tag_name": "button",
                    "name": "Save",
                    "text": "Save",
                    "attributes": { "data-testid": "save-settings" },
                    "is_visible": true,
                    "is_interactive": true,
                    "ax_ref": "browser-use-rs-1"
                }],
                "eval_tree": {
                    "node_type": "element",
                    "tag_name": "body",
                    "is_visible": true,
                    "children": [{
                        "node_type": "element",
                        "tag_name": "button",
                        "attributes": { "data-testid": "save-settings" },
                        "is_visible": true,
                        "is_interactive": true,
                        "ax_ref": "browser-use-rs-1",
                        "children": [{
                            "node_type": "text",
                            "node_value": "Save"
                        }]
                    }]
                }
            }),
            &accessibility,
        )
        .expect("dom state");

        assert_eq!(
            state.eval_representation(),
            "<body />\n\t[i_55] <button data-testid=\"save-settings\" ax_name=\"Save settings\" ax_description=\"Persists account settings\">Save"
        );
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
    fn interactive_snapshot_indexes_all_tabindex_values_like_upstream() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[tabindex]"));
        assert!(!INTERACTIVE_ELEMENTS_JS.contains("[tabindex]:not"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("[tabindex]"));
        assert!(!action_script.contains("[tabindex]:not"));
    }

    #[test]
    fn interactive_snapshot_detects_aria_interactivity_properties() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasAriaInteractivityProperty"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-required"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-autocomplete"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-keyshortcuts"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasAriaInteractivityProperty"));
        assert!(action_script.contains("autocomplete !== 'none'"));
        assert!(action_script.contains("aria-keyshortcuts"));
    }

    #[test]
    fn interactive_snapshot_detects_contenteditable_variants() {
        let contenteditable_selector = r#"[contenteditable]:not([contenteditable="false"])"#;
        assert!(INTERACTIVE_ELEMENTS_JS.contains(contenteditable_selector));

        let action_script = click_element_js(1);
        assert!(action_script.contains(contenteditable_selector));
    }

    #[test]
    fn interactive_snapshot_indexes_anchor_tags_without_href() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'a',"));
        assert!(!INTERACTIVE_ELEMENTS_JS.contains("'a[href]'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("'a',"));
        assert!(!action_script.contains("'a[href]'"));
    }

    #[test]
    fn interactive_snapshot_filters_occluded_elements() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isTopmostAtCenter"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elementFromPoint"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("root.host"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("const paintOrderFiltering = true;"));
        assert!(
            INTERACTIVE_ELEMENTS_JS.contains("(!paintOrderFiltering || isTopmostAtCenter(el))")
        );

        let action_script = click_element_js(1);
        assert!(action_script.contains("isTopmostAtCenter"));
        assert!(action_script.contains("elementFromPoint"));
    }

    #[test]
    fn interactive_snapshot_script_carries_paint_order_filtering_control() {
        let config = IframeTraversalConfig::from_profile(&BrowserProfile::default());
        let enabled = interactive_elements_js(config, true);
        assert!(enabled.contains("const paintOrderFiltering = true;"));
        assert!(enabled.contains("(!paintOrderFiltering || isTopmostAtCenter(el))"));

        let disabled = interactive_elements_js(config, false);
        assert!(disabled.contains("const paintOrderFiltering = false;"));
        assert!(disabled.contains("(!paintOrderFiltering || isTopmostAtCenter(el))"));
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
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elements: indexedElements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("eval_tree: evalTreeForElement"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("node_type: 'document_fragment'"));
    }

    #[test]
    fn interactive_snapshot_indexes_scrollable_containers_without_descendant_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shouldIndexScrollable"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasInteractiveDescendant"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isDropdownContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("scrollInfoText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.scroll = scroll"));

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
    fn executable_resolution_prefers_env_before_candidates() {
        let env_exe = std::env::current_exe().expect("current exe");
        let candidate = PathBuf::from("/definitely/not/a/channel-browser");
        let resolved = resolve_chrome_executable(None, Some(env_exe.clone()), vec![candidate])
            .expect("resolve executable from env");

        assert_eq!(resolved, env_exe);
    }

    #[test]
    fn browser_channel_candidates_are_channel_specific() {
        let beta_candidates = browser_channel_candidates(BrowserChannel::ChromeBeta);
        assert!(!beta_candidates.is_empty());
        assert_eq!(
            browser_executable_candidates(Some(BrowserChannel::ChromeBeta)),
            beta_candidates
        );
        assert_eq!(
            browser_executable_candidates(None),
            default_chrome_candidates()
        );

        let beta_candidate_text = beta_candidates
            .iter()
            .map(|path| path.display().to_string().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            beta_candidate_text.contains("beta"),
            "chrome-beta candidates should be beta-specific: {beta_candidates:?}"
        );
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
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("#shadow"),
            "eval tree missed shadow root: {eval}"
        );
        assert!(
            eval.contains("[i_") && eval.contains("Shadow click"),
            "eval tree missed backend-indexed shadow control: {eval}"
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
    async fn cdp_session_can_index_and_act_in_cross_origin_iframe_targets() {
        let child_html = "<html><body><button id='child-button' onclick=\"document.body.dataset.clicked='yes'\">Cross child</button><input id='child-input' placeholder='Cross input'></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross iframe smoke</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross child") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross child"))
            .expect("child button");
        let child_input = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross input"))
            .expect("child input");
        assert_ne!(child_button.target_id, iframe.target_id);
        assert_ne!(child_input.target_id, iframe.target_id);
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("#iframe-content"),
            "cross-origin eval tree missed iframe content marker: {eval}"
        );
        assert!(
            eval.contains("Cross child"),
            "cross-origin eval tree missed child target content: {eval}"
        );

        session
            .click(child_button.index)
            .await
            .expect("cross-origin child click");
        session
            .input_text(child_input.index, "EvalOps", true)
            .await
            .expect("cross-origin child input");

        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");
        let clicked = session
            .evaluate_json_for_page(&child_page.page, "document.body.dataset.clicked", false)
            .await
            .expect("child clicked flag");
        let input_value = session
            .evaluate_json_for_page(
                &child_page.page,
                "document.getElementById('child-input').value",
                false,
            )
            .await
            .expect("child input value");

        assert_eq!(clicked.as_str(), Some("yes"));
        assert_eq!(input_value.as_str(), Some("EvalOps"));
        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_cross_origin_iframe_target_for_stale_node_fallback() {
        let child_html = "<html><body><button id='child-button' onclick=\"document.body.dataset.clicked='initial'\">Cross child</button><input id='child-input' placeholder='Cross input'></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross stale fallback</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross child") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "child-button")
            })
            .expect("child button")
            .clone();
        let child_input = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "child-input")
            })
            .expect("child input")
            .clone();
        assert_ne!(child_button.target_id, iframe.target_id);
        assert_eq!(child_button.target_id, child_input.target_id);
        assert!(child_button.index > 1);
        assert!(child_input.index > 1);

        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");
        session
            .evaluate_json_for_page(
                &child_page.page,
                r#"
(() => {
  document.open();
  document.write(`<html><body><button id="child-button" onclick="document.body.dataset.clicked='replacement'">Replacement child</button><input id="child-input" placeholder="Replacement input"></body></html>`);
  document.close();
  return true;
})()
"#,
                false,
            )
            .await
            .expect("replace child document");
        sleep(Duration::from_millis(100)).await;

        session
            .click(child_button.index)
            .await
            .expect("click replacement child through fallback");
        session
            .input_text(child_input.index, "EvalOps", true)
            .await
            .expect("input replacement child through fallback");

        let values = session
            .evaluate_json_for_page(
                &child_page.page,
                "JSON.stringify({ clicked: document.body.dataset.clicked || '', input: document.getElementById('child-input').value || '' })",
                false,
            )
            .await
            .expect("child values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded child values"))
            .expect("child values json");
        assert_eq!(values["clicked"].as_str(), Some("replacement"));
        assert_eq!(values["input"].as_str(), Some("EvalOps"));

        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_detached_cached_node_falls_back_inside_cross_origin_iframe_target() {
        let child_html = r#"<html><body><button id='child-button' onclick="document.body.dataset.clicked='old'">Cross stale</button><script>
function replaceChildButton() {
  const next = document.createElement('button');
  next.id = 'child-button';
  next.textContent = 'Cross stale';
  next.onclick = () => { document.body.dataset.clicked = 'replacement'; };
  document.getElementById('child-button').replaceWith(next);
}
</script></body></html>"#;
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross iframe detached fallback</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross stale") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross stale"))
            .expect("child button");
        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");

        session
            .evaluate_json_for_page(&child_page.page, "replaceChildButton(); true", false)
            .await
            .expect("replace cached child button");
        session
            .click(child_button.index)
            .await
            .expect("fallback child click");
        let clicked = session
            .evaluate_json_for_page(&child_page.page, "document.body.dataset.clicked", false)
            .await
            .expect("child clicked flag");

        assert_eq!(clicked.as_str(), Some("replacement"));
        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_extracts_text_and_elements_from_cross_origin_iframe_targets() {
        let child_html = "<html><body><p>Frame only text</p><a id='child-link' href='https://example.com/frame'>Frame link</a></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross extract smoke</title></head><body><p>Parent only text</p><a id='parent-link' href='https://example.com/parent'>Parent link</a><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut page_text = String::new();
        for _ in 0..20 {
            page_text = session.page_text().await.expect("page text");
            if page_text.contains("Frame only text") {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            page_text.contains("Parent only text"),
            "missing parent text: {page_text}"
        );
        assert!(
            page_text.contains("Frame only text"),
            "missing child frame text: {page_text}"
        );

        let links = session
            .find_elements("a", &["href".to_owned()], 10, true)
            .await
            .expect("find links");
        assert!(
            links.iter().any(|link| {
                link.text.as_deref() == Some("Parent link")
                    && link.attributes.get("href").map(String::as_str)
                        == Some("https://example.com/parent")
            }),
            "missing parent link: {links:?}"
        );
        assert!(
            links.iter().any(|link| {
                link.text.as_deref() == Some("Frame link")
                    && link.attributes.get("href").map(String::as_str)
                        == Some("https://example.com/frame")
            }),
            "missing child frame link: {links:?}"
        );

        child_server.abort();
        parent_server.abort();
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
                "data:text/html,<html><head><title>ax props smoke</title></head><body><button id='toggle' aria-expanded='true'>Details</button><div id='slider' role='slider' aria-valuemin='0' aria-valuemax='10' aria-valuenow='7' aria-valuetext='Seven'>Volume</div><div id='results' role='listbox' aria-busy='true' aria-live='polite' aria-level='2' aria-multiselectable='true'>Results</div></body></html>",
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
        assert!(
            llm.contains("valuetext=Seven"),
            "DOM state did not include human-readable value text: {llm}"
        );
        assert!(
            llm.contains("valuemin=0") && llm.contains("valuemax=10") && llm.contains("valuenow=7"),
            "DOM state did not include AX-shaped numeric value metadata: {llm}"
        );
        assert!(
            !llm.contains("aria-valuenow=7"),
            "DOM state did not prefer AX-shaped value aliases over aria value attributes: {llm}"
        );
        assert!(
            llm.contains("busy=true") || llm.contains("busy=1"),
            "DOM state did not include busy live-region state: {llm}"
        );
        assert!(
            llm.contains("live=polite"),
            "DOM state did not include live-region politeness: {llm}"
        );
        assert!(
            llm.contains("level=2"),
            "DOM state did not include hierarchy level: {llm}"
        );
        assert!(
            llm.contains("multiselectable=true"),
            "DOM state did not include multiselectable state: {llm}"
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
    async fn cdp_session_indexes_anchor_without_href() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>plain anchor smoke</title></head><body><a id='plain-anchor'>Plain Anchor</a><a id='href-anchor' href='/target'>Href Anchor</a><button id='button'>Button</button></body></html>",
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

        let plain_anchor = element_by_id("plain-anchor");
        assert_eq!(plain_anchor.tag_name, "a");
        assert_eq!(plain_anchor.name.as_deref(), Some("Plain Anchor"));
        assert!(!plain_anchor.attributes.contains_key("href"));

        let href_anchor = element_by_id("href-anchor");
        assert_eq!(href_anchor.tag_name, "a");
        assert_eq!(
            href_anchor.attributes.get("href").map(String::as_str),
            Some("/target")
        );

        session
            .click(plain_anchor.index)
            .await
            .expect("click plain anchor by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_aria_interactivity_properties() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>aria property smoke</title></head><body><div id='required-proxy' aria-required='true' aria-label='Required proxy'>Required</div><div id='autocomplete-proxy' aria-autocomplete='list' aria-label='Autocomplete proxy'>Autocomplete</div><div id='shortcut-proxy' aria-keyshortcuts='Alt+S' aria-label='Shortcut proxy'>Shortcut</div><div id='autocomplete-none' aria-autocomplete='none'>Ignored autocomplete</div></body></html>",
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

        let required = element_by_id("required-proxy");
        assert_eq!(required.name.as_deref(), Some("Required proxy"));
        assert_eq!(
            required.attributes.get("aria-required").map(String::as_str),
            Some("true")
        );

        let autocomplete = element_by_id("autocomplete-proxy");
        assert_eq!(
            autocomplete
                .attributes
                .get("aria-autocomplete")
                .map(String::as_str),
            Some("list")
        );

        let shortcut = element_by_id("shortcut-proxy");
        assert_eq!(
            shortcut
                .attributes
                .get("aria-keyshortcuts")
                .map(String::as_str),
            Some("Alt+S")
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "autocomplete-none")
        }));

        session
            .click(shortcut.index)
            .await
            .expect("click shortcut proxy by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_contenteditable_variants() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>contenteditable smoke</title></head><body><div id='plain-editor' contenteditable='plaintext-only' aria-label='Plain editor'>Draft</div><div id='true-editor' contenteditable='true' aria-label='True editor'>Rich</div><div id='false-editor' contenteditable='false' aria-label='False editor'>Ignored</div></body></html>",
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
                .unwrap_or_else(|| panic!("missing contenteditable element with id {id}"))
        };

        let plain = element_by_id("plain-editor");
        assert_eq!(plain.name.as_deref(), Some("Plain editor"));
        assert_eq!(
            plain.attributes.get("contenteditable").map(String::as_str),
            Some("plaintext-only")
        );

        let rich = element_by_id("true-editor");
        assert_eq!(rich.name.as_deref(), Some("True editor"));
        assert_eq!(
            rich.attributes.get("contenteditable").map(String::as_str),
            Some("true")
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "false-editor")
        }));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_media_controls() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>media smoke</title></head><body><audio id='audio-player' controls aria-label='Audio sample' style='width:320px'></audio><video id='video-player' controls aria-label='Video sample' width='320' height='180'></video><audio id='silent-audio' aria-label='Silent sample'></audio></body></html>",
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
                .unwrap_or_else(|| panic!("missing media element with id {id}"))
        };

        let audio = element_by_id("audio-player");
        assert_eq!(audio.tag_name, "audio");
        assert!(
            audio
                .attributes
                .get("compound_components")
                .is_some_and(|value| value.contains("Play/Pause") && value.contains("Volume"))
        );

        let video = element_by_id("video-player");
        assert_eq!(video.tag_name, "video");
        assert!(
            video
                .attributes
                .get("compound_components")
                .is_some_and(|value| value.contains("Fullscreen"))
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "silent-audio")
        }));
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
    async fn cdp_session_indexes_negative_tabindex_like_upstream() {
        let profile = BrowserProfile {
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>tabindex smoke</title></head><body><div id='negative-tabindex' tabindex='-1' style='width:140px;height:32px'>Programmatic focus target</div><div id='plain-tabindex' tabindex='0' style='width:140px;height:32px'>Keyboard focus target</div><div id='plain-div' style='width:140px;height:32px'>Plain div</div></body></html>",
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
        for expected in ["negative-tabindex", "plain-tabindex"] {
            assert!(
                ids.contains(&expected),
                "DOM state did not index {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        assert!(
            !ids.contains(&"plain-div"),
            "plain div should not be indexed: {}",
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

        let state = session
            .state(false)
            .await
            .expect("state after blocked preflight navigation");
        assert!(
            state
                .recent_events
                .as_deref()
                .is_some_and(|events| events.contains("no browser navigation was started")),
            "blocked preflight diagnostics missing from state: {state:?}"
        );
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
    async fn cdp_session_watchdog_closes_disallowed_unsolicited_new_tab_before_state() {
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

        sleep(Duration::from_millis(500)).await;
        let tabs = page_tabs(&session.connection)
            .await
            .expect("tabs after watchdog enforcement");
        assert!(
            tabs.iter().all(|tab| tab.target_id != blocked_target_id),
            "blocked tab still open before state/action boundary: {tabs:?}"
        );

        let error = session
            .state(false)
            .await
            .expect_err("state observation should report watchdog-blocked tab");

        assert!(
            matches!(
                error,
                BrowserError::NavigationBlocked { ref url, ref reason }
                    if url.starts_with("http://127.0.0.1:1/popup")
                        && reason == "ip_address_blocked"
            ),
            "unexpected blocked popup policy error: {error:?}"
        );

        let state = session
            .state(false)
            .await
            .expect("state after watchdog policy error was reported");
        assert!(
            state
                .closed_popup_messages
                .iter()
                .any(|message| message.contains("http://127.0.0.1:1/popup")),
            "closed popup diagnostics missing from state: {state:?}"
        );
        assert!(
            state
                .recent_events
                .as_deref()
                .is_some_and(|events| events.contains("Closed popup")),
            "recent security events missing from state: {state:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_rejects_disallowed_new_tab_from_coordinate_click_action() {
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
                "data:text/html,<html><head><title>blocked click</title></head><body style='margin:0'><button id='blocked' onclick=\"window.open('http://127.0.0.1:1/popup')\" style='position:absolute;left:20px;top:20px;width:180px;height:44px'>Blocked popup</button></body></html>",
                false,
            )
            .await
            .expect("navigate allowed data page");
        sleep(Duration::from_millis(100)).await;

        let error = session
            .click_coordinates(40, 40)
            .await
            .expect_err("coordinate click should enforce blocked popup policy");

        assert!(
            matches!(
                error,
                BrowserError::NavigationBlocked { ref url, ref reason }
                    if url.starts_with("http://127.0.0.1:1/popup")
                        && reason == "ip_address_blocked"
            ),
            "unexpected blocked popup policy error: {error:?}"
        );
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
        assert!(
            plain_pane
                .attributes
                .get("scroll")
                .is_some_and(|value| value.contains("pages below")),
            "plain pane was missing scroll context: {plain_pane:?}"
        );
        assert!(
            state.dom_state.llm_representation().contains("pages below"),
            "DOM state did not render scroll context: {}",
            state.dom_state.llm_representation()
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
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("<html"),
            "eval tree missed document root: {eval}"
        );
        assert!(
            eval.contains("[i_") && eval.contains("Click me"),
            "eval tree missed backend-indexed button: {eval}"
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
