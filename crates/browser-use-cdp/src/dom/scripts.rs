use super::*;

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
