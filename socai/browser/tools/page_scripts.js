const SocaiBrowserPageScripts = (() => {
  function measureElement(el) {
    const rect = el.getBoundingClientRect();
    const visible = rect.width > 0 && rect.height > 0
      && rect.bottom > 0 && rect.right > 0
      && rect.top < (window.innerHeight || document.documentElement.clientHeight)
      && rect.left < (window.innerWidth || document.documentElement.clientWidth);
    return {
      visible,
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      x: Math.round(rect.left + rect.width / 2),
      y: Math.round(rect.top + rect.height / 2),
    };
  }

  function clickSelector(arg) {
    const selector = String((arg && arg.selector) || '');
    const el = document.querySelector(selector);
    if (!el) return { ok: false, error: 'selector_not_found' };
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const measure = measureElement(el);
    if (measure.width <= 0 || measure.height <= 0) {
      return { ok: false, error: 'zero_sized_element' };
    }
    return {
      ok: true,
      x: measure.x,
      y: measure.y,
      width: measure.width,
      height: measure.height,
      visible: measure.visible,
    };
  }

  function fillSelector(arg) {
    const selector = String((arg && arg.selector) || '');
    const el = document.querySelector(selector);
    if (!el) return { ok: false, error: 'selector_not_found' };
    if (!(el instanceof HTMLElement)) return { ok: false, error: 'not_html_element' };
    el.scrollIntoView({ block: 'center', inline: 'center' });
    el.focus();
    if ('value' in el) {
      try { el.value = ''; } catch (e) {}
      el.dispatchEvent(new Event('input', { bubbles: true }));
    } else if (el.isContentEditable) {
      el.textContent = '';
    }
    const measure = measureElement(el);
    return { ok: true, x: measure.x, y: measure.y };
  }

  function waitForSelector(arg) {
    const selector = String((arg && arg.selector) || '');
    const requireVisible = !(arg && arg.require_visible === false);
    const timeoutMs = Math.max(100, Number((arg && arg.timeout_ms) || 8000));

    return new Promise((resolve) => {
      const measure = (el) => ({ found: true, ...measureElement(el) });
      const found = document.querySelector(selector);
      if (found && (!requireVisible || measure(found).visible)) {
        resolve(measure(found));
        return;
      }

      let settled = false;
      let observer = null;
      let timer = null;
      const finish = (value) => {
        if (settled) return;
        settled = true;
        if (observer) observer.disconnect();
        if (timer) clearTimeout(timer);
        resolve(value);
      };

      observer = new MutationObserver(() => {
        const el = document.querySelector(selector);
        if (!el) return;
        if (requireVisible && !measure(el).visible) return;
        finish(measure(el));
      });
      observer.observe(document.body || document.documentElement, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ['class', 'style', 'hidden', 'aria-hidden'],
      });
      timer = setTimeout(() => finish({ found: false }), timeoutMs);
    });
  }

  return { clickSelector, fillSelector, waitForSelector };
})();
