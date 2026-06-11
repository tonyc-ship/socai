const SocaiDyPageScripts = (() => {
  const $ = (sel, root = document) => (root || document).querySelector(sel);
  const $$ = (sel, root = document) => Array.from((root || document).querySelectorAll(sel));
  const text = (el) => (el ? (el.innerText || el.textContent || '').trim() : '');
  const norm = (s) => String(s || '')
    .replace(/\u00a0/g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  const absUrl = (url) => {
    try { return url ? new URL(url, location.href).href : ''; } catch (e) { return ''; }
  };

  function isVisible(el) {
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return false;
    const style = window.getComputedStyle(el);
    return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
  }

  function elementCenter(el) {
    const rect = el.getBoundingClientRect();
    return {
      x: Math.round(rect.left + rect.width / 2),
      y: Math.round(rect.top + rect.height / 2),
    };
  }

  const SEARCH_INPUT_SELECTORS = [
    'input[data-e2e*="search"]',
    'input[placeholder*="搜索"]',
    'input[type="search"]',
    'textarea[placeholder*="搜索"]',
    '[contenteditable="true"][data-e2e*="search"]',
    '[contenteditable="true"][role="searchbox"]',
    '[role="searchbox"] input',
    'form input',
  ];

  function findSearchInput() {
    for (const sel of SEARCH_INPUT_SELECTORS) {
      for (const el of $$(sel)) {
        if (!(el instanceof HTMLElement)) continue;
        if (!isVisible(el)) continue;
        if (el.getBoundingClientRect().width >= 80) return el;
      }
    }
    return null;
  }

  const SEARCH_SUBMIT_SELECTORS = [
    'button[type="submit"]',
    '[data-e2e*="search"] button',
    '[aria-label*="搜索"]',
    '[class*="search"] button',
    'svg[class*="search"]',
  ];

  function searchInput() {
    const input = findSearchInput();
    if (!input) return { ok: false, error: 'search_input_not_found' };
    const root = input.closest('form, header, [role="search"], [class*="search"]') || document;
    let submit = null;
    for (const sel of SEARCH_SUBMIT_SELECTORS) {
      const el = root.querySelector(sel) || document.querySelector(sel);
      if (!el || !isVisible(el)) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width < 10 || rect.height < 10) continue;
      submit = elementCenter(el);
      break;
    }
    return {
      ok: true,
      input: elementCenter(input),
      submit,
      value: input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement
        ? input.value
        : text(input),
    };
  }

  function setSearchInput(arg) {
    const targetValue = String((arg && arg.query) || '');
    const input = findSearchInput();
    if (!input) return { ok: false, error: 'search_input_not_found' };
    input.focus();

    if (input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement) {
      const proto = input instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
      const descriptor = Object.getOwnPropertyDescriptor(proto, 'value');
      if (descriptor && descriptor.set) descriptor.set.call(input, targetValue);
      else input.value = targetValue;
    } else if (input.isContentEditable) {
      input.textContent = targetValue;
    } else {
      return { ok: false, error: 'unsupported_search_input' };
    }

    input.dispatchEvent(new InputEvent('input', {
      bubbles: true,
      inputType: 'insertReplacementText',
      data: targetValue,
    }));
    input.dispatchEvent(new Event('change', { bubbles: true }));

    const actualValue = input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement
      ? input.value
      : input.textContent;
    const trimmed = String(actualValue || '').trim();
    return { ok: trimmed === targetValue.trim(), value: trimmed };
  }

  function hasLoginModal() {
    const body = text(document.body);
    const visibleLogin = $$('[class*="login"], [data-e2e*="login"]').some((el) => {
      if (!isVisible(el)) return false;
      return /登录|扫码|验证码|手机号/.test(text(el));
    });
    return visibleLogin || /登录后|扫码登录|手机号登录|验证码登录/.test(body);
  }

  function currentStateName() {
    const url = location.href;
    if (/douyin\.com\/video\//.test(url)) return 'video_detail';
    if (/douyin\.com\/(?:jingxuan\/)?search\//.test(url)) return 'search_results';
    if (/douyin\.com/.test(url)) return 'homepage';
    return 'unknown';
  }

  function videoIdFromUrl(url) {
    const match = String(url || '').match(/\/video\/([^/?#]+)/);
    return match ? match[1] : '';
  }

  function videoIdFromCard(card) {
    const explicit = card.dataset?.awemeId || card.getAttribute('data-aweme-id') || '';
    if (explicit) return explicit;
    const match = String(card.id || '').match(/^waterfall_item_(\d{15,})$/);
    return match ? match[1] : '';
  }

  function closestCard(link) {
    const stable = link.closest('[data-aweme-id], .search-result-card, .discover-video-card-item');
    if (stable) return stable;
    let node = link;
    for (let i = 0; node && i < 6; i += 1, node = node.parentElement) {
      if (!(node instanceof HTMLElement)) continue;
      const rect = node.getBoundingClientRect();
      if (rect.width >= 120 && rect.height >= 120) return node;
    }
    return link;
  }

  function firstImageUrl(root) {
    const img = $('img[src], img[data-src]', root);
    if (img) return absUrl(img.currentSrc || img.src || img.getAttribute('data-src') || '');
    const source = $('source[src], source[srcset]', root);
    if (source) {
      const value = source.getAttribute('src') || String(source.getAttribute('srcset') || '').split(/\s+/)[0];
      return absUrl(value);
    }
    return '';
  }

  function titleFor(link, card, raw) {
    const direct = [
      link.getAttribute('aria-label'),
      link.getAttribute('title'),
      link.title,
      text($('[title]', card)),
      text($('.t8', card)),
      text($('[class*="line-clamp"]', card)),
      text($('[class*="title"]', card)),
      text($('[data-e2e*="title"]', card)),
    ].find((value) => String(value || '').trim());
    if (direct) return norm(direct);
    return norm(raw.split('\n').find((line) => line.trim().length >= 4) || raw).slice(0, 240);
  }

  function metric(raw, labels) {
    const lines = raw.split('\n').map((line) => line.trim()).filter(Boolean);
    for (const label of labels) {
      const re = new RegExp(`${label}[:：]?\\s*([\\d.,万wWkK+]+)`);
      const found = raw.match(re);
      if (found) return found[1];
    }
    for (let i = 0; i < lines.length - 1; i += 1) {
      if (labels.some((label) => lines[i].includes(label)) && /[\d万wWkK]/.test(lines[i + 1])) {
        return lines[i + 1];
      }
    }
    return '';
  }

  function firstCountLike(raw) {
    const lines = raw.split('\n').map((line) => line.trim()).filter(Boolean);
    for (const line of lines) {
      if (/^\d[\d.,]*(?:万|w|W|k|K)?$/.test(line)) return line;
    }
    return '';
  }

  function authorFor(card, raw) {
    const authorEl = $('a[href*="/user/"], a[href*="/discover/user"], [data-e2e*="author"], .lGzJpEad', card);
    if (authorEl) {
      return {
        author: norm(text(authorEl)).replace(/^@/, ''),
        author_url: absUrl(authorEl.href || authorEl.getAttribute('href') || ''),
      };
    }
    const match = raw.match(/@\\s*([^\\n·]+)/);
    return {
      author: match ? norm(match[1]) : '',
      author_url: '',
    };
  }

  function candidateCards() {
    const out = [];
    const push = (el, preferSelf = false) => {
      if (!(el instanceof HTMLElement)) return;
      const card = preferSelf || el.matches('.search-result-card')
        ? el
        : el;
      if (!card || out.includes(card)) return;
      out.push(card);
    };
    const searchCards = $$('.search-result-card');
    if (searchCards.length) {
      searchCards.forEach((el) => push(el, true));
      return out;
    }
    $$('[data-aweme-id]').forEach(push);
    $$('.search-result-card, .discover-video-card-item').forEach(push);
    $$('a[href*="/video/"]').forEach((link) => push(closestCard(link)));
    return out;
  }

  function videoCards() {
    const seen = new Set();
    const cards = [];
    for (const card of candidateCards()) {
      const host = card.closest('[id^="waterfall_item_"], [data-aweme-id]') || card;
      const link = $('a[href*="/video/"]', host) || (host.matches('a[href*="/video/"]') ? host : null);
      const cardId = videoIdFromCard(host);
      const url = link ? absUrl(link.href || link.getAttribute('href')) : (cardId ? `https://www.douyin.com/video/${cardId}` : '');
      const videoId = videoIdFromUrl(url) || cardId;
      const key = videoId || url;
      if (!key || seen.has(key)) continue;
      if (!link && !$('.videoImage, img[src], img[data-src]', host)) continue;
      seen.add(key);
      const raw = norm(text(host));
      const author = authorFor(host, raw);
      cards.push({
        video_id: videoId,
        title: titleFor(link || host, host, raw),
        author: author.author,
        author_url: author.author_url,
        url,
        cover_url: firstImageUrl(host),
        likes: metric(raw, ['点赞', '赞']) || firstCountLike(raw),
        comments: metric(raw, ['评论']),
        shares: metric(raw, ['分享', '转发']),
        duration: (raw.match(/\b\d{1,2}:\d{2}(?::\d{2})?\b/) || [''])[0],
        raw_text: raw.slice(0, 1200),
        position: cards.length,
      });
    }
    return cards;
  }

  function searchState() {
    const input = findSearchInput();
    const body = text(document.body);
    const cards = videoCards();
    const url = new URL(location.href);
    return {
      ok: true,
      page_state: currentStateName(),
      url: location.href,
      tab_type: url.searchParams.get('type') || '',
      input_keyword: input
        ? String(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement ? input.value : input.textContent || '').trim()
        : '',
      card_count: cards.length,
      loading: $$('[class*="loading"], [class*="spinner"]').some(isVisible),
      has_no_results: /暂无|没有找到|无结果|换个词试试|no result/i.test(body),
      login_required: hasLoginModal(),
    };
  }

  function searchTabs() {
    const labelsByKey = {
      general: '综合',
      video: '视频',
      user: '用户',
      live: '直播',
    };
    const tabs = [];
    const seen = new Set();
    for (const el of $$('[data-key]')) {
      if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
      const keyName = el.getAttribute('data-key') || '';
      const label = labelsByKey[keyName] || norm(text(el));
      if (!Object.values(labelsByKey).includes(label)) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width < 20 || rect.height < 16 || rect.top > 260) continue;
      tabs.push({
        label,
        active: String(el.className || '').includes('wEOX') || (keyName === 'video' && new URL(location.href).searchParams.get('type') === 'video'),
        x: Math.round(rect.left + rect.width / 2),
        y: Math.round(rect.top + rect.height / 2),
      });
    }
    if (tabs.length) return tabs;

    const labels = Object.values(labelsByKey);
    for (const el of $$('a, button, [role="tab"], [role="button"], div, span')) {
      if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
      const label = norm(text(el));
      if (!labels.includes(label)) continue;
      const clickable = el.closest('a, button, [role="tab"], [role="button"]') || el;
      const rect = clickable.getBoundingClientRect();
      if (rect.width < 20 || rect.height < 16 || rect.top > 260) continue;
      const key = `${label}:${Math.round(rect.left)}:${Math.round(rect.top)}`;
      if (seen.has(key)) continue;
      seen.add(key);
      tabs.push({
        label,
        active: label === '视频'
          ? new URL(location.href).searchParams.get('type') === 'video'
          : label === '综合' && (new URL(location.href).searchParams.get('type') || 'general') === 'general',
        x: Math.round(rect.left + rect.width / 2),
        y: Math.round(rect.top + rect.height / 2),
      });
    }
    return tabs;
  }

  function clickSearchTab(arg) {
    const label = String(arg || '');
    const tabs = searchTabs();
    const target = tabs.find((tab) => tab.label === label);
    if (!target) return { ok: false, error: 'tab_not_found', label, tabs };
    return { ok: true, ...target, tabs };
  }

  function pageState() {
    return {
      ok: true,
      state: currentStateName(),
      url: location.href,
      title: document.title,
      search: searchState(),
      search_input: searchInput(),
      login_required: hasLoginModal(),
    };
  }

  function scrollFeed(arg) {
    const before = {
      x: scrollX,
      y: scrollY,
      height: document.documentElement.scrollHeight,
      cards: videoCards().length,
    };
    const nudgeUp = !!(arg && arg.nudge_up);
    const delta = nudgeUp ? -Math.max(300, Math.round(innerHeight * 0.35)) : Math.max(700, Math.round(innerHeight * 0.9));
    window.scrollBy({ left: 0, top: delta, behavior: 'instant' });
    return {
      ok: true,
      before,
      after: {
        x: scrollX,
        y: scrollY,
        height: document.documentElement.scrollHeight,
        cards: videoCards().length,
      },
    };
  }

  return {
    pageState,
    searchInput,
    setSearchInput,
    searchState,
    searchTabs,
    clickSearchTab,
    videoCards,
    scrollFeed,
  };
})();
