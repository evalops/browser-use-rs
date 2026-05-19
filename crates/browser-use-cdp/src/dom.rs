//! DOM, accessibility, and page-state extraction over CDP.
//!
//! This module is the bridge from raw Chrome JSON to prompt-ready
//! [`browser_use_dom`] structures. It evaluates browser scripts, gathers
//! accessibility metadata, merges iframe/shadow states, highlights elements,
//! detects pagination, and keeps selector-map indexes stable enough for action
//! execution and replay.

use crate::{
    AttachedPage, BrowserError, CachedDomElementRef, FrameElementInfo, FrameOffset,
    IframeTargetInfo, IframeTraversalConfig,
};
use browser_use_dom::{
    DomElementRef, DomEvalNode, DomEvalNodeType, DomPageStats, ElementBounds, PageInfo,
    PaginationButton, PaginationButtonType, SerializedDomState, render_element_text,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

pub(crate) const AX_REF_ATTRIBUTE: &str = "data-browser-use-rs-ax-ref";

pub(crate) const INTERACTIVE_ELEMENTS_JS: &str = r#"
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
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    const isNativeMediaControl = (tag === 'audio' || tag === 'video') && el.hasAttribute('controls');
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return !isDisabledOrHidden(el) && rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden' && (!paintOrderFiltering || isNativeMediaControl || isTopmostAtCenter(el));
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

pub(crate) const CLEANUP_AX_REFS_JS: &str = r#"
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

pub(crate) const FRAME_ELEMENTS_JS: &str = r#"
JSON.stringify(Array.from(document.querySelectorAll('iframe,frame')).map((el) => {
  const rect = el.getBoundingClientRect();
  return {
    url: el.src || el.getAttribute('src') || '',
    x: Math.round(rect.x),
    y: Math.round(rect.y)
  };
}))
"#;

pub(crate) const PAGE_INFO_JS: &str = r#"
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

pub(crate) fn interactive_elements_js(
    config: IframeTraversalConfig,
    paint_order_filtering: bool,
) -> String {
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AccessibilityNodeInfo {
    pub(crate) backend_node_id: u64,
    pub(crate) node_id: Option<u64>,
    pub(crate) role: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) properties: BTreeMap<String, String>,
}

pub(crate) fn dom_state_from_interactive_value(
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

pub(crate) fn frame_element_infos_from_value(
    value: &Value,
) -> Result<Vec<FrameElementInfo>, BrowserError> {
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

pub(crate) fn iframe_target_infos_from_targets(
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

pub(crate) fn frame_offset_for_target_url(
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

pub(crate) fn offset_dom_state_bounds(state: &mut SerializedDomState, offset: FrameOffset) {
    for element in state.selector_map.values_mut() {
        if let Some(bounds) = &mut element.bounds {
            bounds.x += offset.x;
            bounds.y += offset.y;
        }
    }
}

pub(crate) fn merge_dom_states(
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

pub(crate) fn dom_state_elements(state: SerializedDomState) -> Vec<DomElementRef> {
    state.selector_map.into_values().collect()
}

pub(crate) fn target_local_index_for_global_index(
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

pub(crate) fn index_fallback_target_id<'a>(
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

pub(crate) fn add_dom_page_stats(total: &mut DomPageStats, next: DomPageStats) {
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

pub(crate) fn dom_element_from_value(
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

pub(crate) fn snapshot_backend_ids_by_ax_ref(snapshot: &Value) -> BTreeMap<String, u64> {
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

pub(crate) fn accessibility_nodes_by_backend_id(
    tree: &Value,
) -> BTreeMap<u64, AccessibilityNodeInfo> {
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

pub(crate) fn enriched_attributes_from_value(
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

pub(crate) fn is_ax_suppressed_interactive_element(element: &DomElementRef) -> bool {
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

pub(crate) fn should_fallback_to_index_traversal(error: &BrowserError) -> bool {
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

pub(crate) fn is_missing_target_error(error: &BrowserError) -> bool {
    matches!(
        error,
        BrowserError::CommandFailed { method, message }
            if matches!(method.as_str(), "Target.attachToTarget" | "Target.closeTarget")
                && message.contains("No target with given id found")
    )
}

pub(crate) fn parse_dropdown_options_value(value: Value) -> Result<Vec<String>, BrowserError> {
    let encoded = value
        .as_str()
        .ok_or_else(|| BrowserError::MissingResponseData("dropdown options string".to_owned()))?;
    serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
}

pub(crate) fn element_bounds_from_value(value: &Value) -> Option<ElementBounds> {
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

pub(crate) fn page_info_from_value(value: &Value) -> Option<PageInfo> {
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

pub(crate) fn detect_pagination_buttons(dom_state: &SerializedDomState) -> Vec<PaginationButton> {
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

pub(crate) fn u32_field(value: &Value, field: &str) -> Option<u32> {
    value
        .get(field)?
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
}

pub(crate) fn i32_field(value: &Value, field: &str) -> Option<i32> {
    value
        .get(field)?
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
}
