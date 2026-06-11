const SocaiDyPageScripts = (() => {
  const $ = (sel, root = document) => (root || document).querySelector(sel);
  const $$ = (sel, root = document) => Array.from((root || document).querySelectorAll(sel));
  const text = (el) => norm(el ? (el.innerText || el.textContent || '') : '');
  const norm = (s) => String(s || '')
    .replace(/ /g, ' ')
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
    '[data-e2e="searchbar-input"]',
    'input[type="search"]',
    'input[placeholder*="搜索"]',
    'input[data-e2e*="search"]',
    '[contenteditable="true"][role="textbox"]',
    '[contenteditable="true"]',
    'input',
  ];

  function findSearchInput() {
    for (const sel of SEARCH_INPUT_SELECTORS) {
      for (const el of $$(sel)) {
        if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
        const rect = el.getBoundingClientRect();
        if (rect.width < 120 || rect.height < 16) continue;
        return el;
      }
    }
    return null;
  }

  function searchInput() {
    const el = findSearchInput();
    if (el) {
      const root = el.closest('form, header, [data-e2e*="search"], [class*="search"]') || document;
      const button = root.querySelector('[data-e2e="searchbar-button"], button, [role="button"]')
        || $('[data-e2e="searchbar-button"]');
      return {
        ok: true,
        input: elementCenter(el),
        submit: button && isVisible(button) ? elementCenter(button) : null,
        tag: el.tagName.toLowerCase(),
        type: el.getAttribute('type') || '',
        placeholder: el.getAttribute('placeholder') || '',
        value: el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement
          ? el.value
          : text(el),
      };
    }
    return { ok: false, error: 'search_input_not_found' };
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
    return {
      ok: String(actualValue || '').trim() === targetValue.trim(),
      value: String(actualValue || '').trim(),
    };
  }

  function videoLinks() {
    const seen = new Set();
    const links = [];
    for (const link of $$('a[href*="/video/"]')) {
      if (!(link instanceof HTMLElement) || !isVisible(link)) continue;
      const href = absUrl(link.getAttribute('href') || link.href || '');
      const match = href.match(/\/video\/(\d+)/);
      if (!match || seen.has(match[1])) continue;
      seen.add(match[1]);
      links.push(link);
    }
    return links;
  }

  function searchResultCardElements() {
    const seen = new Set();
    const cards = [];
    for (const item of $$('[id^="waterfall_item_"]')) {
      if (!(item instanceof HTMLElement) || !isVisible(item)) continue;
      const id = (item.id || '').replace(/^waterfall_item_/, '');
      if (!/^\d+$/.test(id) || seen.has(id)) continue;
      const card = item.querySelector('.search-result-card') || item;
      if (!isVisible(card)) continue;
      seen.add(id);
      cards.push({ item, card, id });
    }
    return cards;
  }

  function cardRootFromLink(link) {
    let best = link;
    let cur = link;
    for (let depth = 0; cur && depth < 10; depth += 1, cur = cur.parentElement) {
      const rect = cur.getBoundingClientRect();
      if (
        rect.width >= 120 && rect.height >= 100 &&
        rect.width <= Math.max(900, innerWidth * 0.7) &&
        rect.height <= Math.max(1100, innerHeight * 1.2) &&
        cur.querySelector('img')
      ) {
        best = cur;
      }
    }
    return best;
  }

  function firstImageUrl(root) {
    const img = root.querySelector('img[src], img[srcset]');
    if (!img) return '';
    const src = img.currentSrc || img.src || '';
    if (src) return absUrl(src);
    const srcset = img.getAttribute('srcset') || '';
    const first = srcset.split(',')[0]?.trim()?.split(/\s+/)[0] || '';
    return absUrl(first);
  }

  function titleFromText(raw) {
    const lines = norm(raw).split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
      .filter((line) => !/^(@|#?\d+\.?\d*[万w]?$|相关搜索|搜索$|综合$|视频$|用户$|直播$)/.test(line));
    return lines[0] || '';
  }

  function searchResultCards() {
    return searchResultCardElements().map(({ item, card, id }, i) => {
      const raw = text(card);
      const date = text(card.querySelector('.Yftofmx6')).replace(/^·\s*/, '');
      return {
        video_id: id,
        title: text(card.querySelector('.RBpYLmIg')) || titleFromText(raw),
        author: text(card.querySelector('.lGzJpEad')),
        author_url: '',
        likes: text(card.querySelector('.GiEcbsyC span')),
        comments: '',
        shares: '',
        collects: '',
        duration: text(card.querySelector('.cxEIO6RG')),
        publish_time: date,
        link: `https://www.douyin.com/video/${id}`,
        cover_url: firstImageUrl(card),
        position: i,
        raw_text: raw || text(item),
      };
    });
  }

  function videoCards() {
    const searchCards = searchResultCards();
    if (searchCards.length) return searchCards;

    const domCards = videoLinks().map((link, i) => {
      const href = absUrl(link.getAttribute('href') || link.href || '');
      const idMatch = href.match(/\/video\/(\d+)/);
      const root = cardRootFromLink(link);
      const raw = text(root);
      const authorMatch = raw.match(/@\s*([^·\n]+)(?:\s*·|\n|$)/);
      const durationMatch = raw.match(/\b\d{1,2}:\d{2}\b/);
      const dateMatch = raw.match(/(?:\d{4}年\d{1,2}月\d{1,2}日|\d{1,2}月\d{1,2}日|昨天|今天|\d+天前)/);
      return {
        video_id: idMatch ? idMatch[1] : '',
        title: titleFromText(link.getAttribute('aria-label') || raw),
        author: authorMatch ? authorMatch[1].trim() : '',
        author_url: '',
        likes: '',
        comments: '',
        shares: '',
        collects: '',
        duration: durationMatch ? durationMatch[0] : '',
        publish_time: dateMatch ? dateMatch[0] : '',
        link: href,
        cover_url: firstImageUrl(root),
        position: i,
        raw_text: raw,
      };
    });
    return domCards.length ? domCards : embeddedAwemeCards();
  }

  function readEscapedField(chunk, key) {
    const re = new RegExp('\\\\\\"' + key + '\\\\\\":\\\\\\"((?:\\\\\\\\.|[^\\\\"])*)\\\\\\"');
    const match = chunk.match(re);
    return match ? decodeEscaped(match[1]) : '';
  }

  function readEscapedNumber(chunk, key) {
    const re = new RegExp('\\\\\\"' + key + '\\\\\\":(null|-?\\d+(?:\\.\\d+)?)');
    const match = chunk.match(re);
    return match && match[1] !== 'null' ? match[1] : '';
  }

  function decodeEscaped(value) {
    if (!value) return '';
    try {
      return JSON.parse('"' + value.replace(/"/g, '\\"') + '"');
    } catch (e) {
      return value
        .replace(/\\u0026/g, '&')
        .replace(/\\"/g, '"')
        .replace(/\\\\/g, '\\');
    }
  }

  function formatDuration(ms) {
    const n = Number(ms || 0);
    if (!Number.isFinite(n) || n <= 0) return '';
    const total = Math.round(n / 1000);
    const minutes = Math.floor(total / 60);
    const seconds = String(total % 60).padStart(2, '0');
    return `${minutes}:${seconds}`;
  }

  function formatDate(seconds) {
    const n = Number(seconds || 0);
    if (!Number.isFinite(n) || n <= 0) return '';
    const date = new Date(n * 1000);
    if (!Number.isFinite(date.getTime())) return '';
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  }

  function embeddedAwemeCards() {
    const html = document.documentElement.innerHTML;
    const seen = new Set();
    const cards = [];
    const re = /\\"awemeId\\":\\"(\d+)\\"/g;
    let match;
    while ((match = re.exec(html)) && cards.length < 200) {
      const id = match[1];
      if (seen.has(id)) continue;
      seen.add(id);

      const objectStart = Math.max(
        0,
        html.lastIndexOf('{\\"entertainmentVideoType\\"', match.index),
        html.lastIndexOf('{\\"awemeId\\"', match.index)
      );
      const nextObject = html.indexOf(',{\\"entertainmentVideoType\\"', match.index + 20);
      const objectEnd = nextObject > match.index ? nextObject : Math.min(html.length, match.index + 50000);
      const chunk = html.slice(objectStart, objectEnd);
      const desc = readEscapedField(chunk, 'desc');
      const authorChunkStart = chunk.indexOf('\\"authorInfo\\"');
      const authorChunk = authorChunkStart >= 0 ? chunk.slice(authorChunkStart, authorChunkStart + 5000) : chunk;
      const videoChunkStart = chunk.indexOf('\\"video\\"');
      const videoChunk = videoChunkStart >= 0 ? chunk.slice(videoChunkStart, videoChunkStart + 12000) : chunk;
      const statsChunkStart = chunk.indexOf('\\"stats\\"');
      const statsChunk = statsChunkStart >= 0 ? chunk.slice(statsChunkStart, statsChunkStart + 2500) : chunk;

      cards.push({
        video_id: id,
        title: desc,
        author: readEscapedField(authorChunk, 'nickname'),
        author_url: '',
        likes: readEscapedNumber(statsChunk, 'diggCount'),
        comments: readEscapedNumber(statsChunk, 'commentCount'),
        shares: readEscapedNumber(statsChunk, 'shareCount'),
        collects: readEscapedNumber(statsChunk, 'collectCount'),
        duration: formatDuration(readEscapedNumber(videoChunk, 'duration')),
        publish_time: formatDate(readEscapedNumber(chunk, 'createTime')),
        link: `https://www.douyin.com/video/${id}`,
        cover_url: readEscapedField(videoChunk, 'cover'),
        position: cards.length,
      });
    }
    return cards;
  }

  function searchState() {
    const input = findSearchInput();
    const url = new URL(location.href);
    const bodyText = text(document.body);
    return {
      ok: true,
      page_state: url.pathname.includes('/search/') ? 'search_results' : 'unknown',
      url: location.href,
      path: url.pathname,
      input_keyword: input
        ? (input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement ? input.value : text(input))
        : '',
      card_count: Math.max(searchResultCardElements().length, videoLinks().length),
      loading: $$('[class*="loading"], [class*="spinner"]').some((el) => isVisible(el)),
      has_no_results: /暂无|没有找到|无结果|换个词试试|no result/i.test(bodyText),
      login_required: /登录后|扫码登录|验证码登录|手机号登录/.test(bodyText),
    };
  }

  function scrollResults(arg) {
    const nudgeUp = !!(arg && arg.nudge_up);
    const before = Math.max(searchResultCardElements().length, videoLinks().length);
    const delta = nudgeUp ? -Math.round(innerHeight * 0.45) : Math.round(innerHeight * 0.85);
    window.scrollBy({ left: 0, top: delta, behavior: 'instant' });
    return {
      ok: true,
      before,
      after: Math.max(searchResultCardElements().length, videoLinks().length),
      scroll_y: scrollY,
      document_height: document.documentElement.scrollHeight,
    };
  }

  function pageState() {
    const url = location.href;
    const bodyText = text(document.body);
    return {
      ok: true,
      state: url.includes('/search/') || /搜索结果/.test(bodyText) ? 'search_results' : 'homepage',
      url,
      title: document.title,
      search_input: searchInput(),
      search: searchState(),
      login_required: /登录后|扫码登录|验证码登录|手机号登录/.test(bodyText),
    };
  }

  return {
    pageState,
    searchInput,
    setSearchInput,
    searchState,
    videoCards,
    scrollResults,
  };
})();
