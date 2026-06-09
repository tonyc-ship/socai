const SocaiXhsPageScripts = (() => {
  // ── tiny DOM helpers ─────────────────────────────────────────
  const $ = (sel, root = document) => (root || document).querySelector(sel);
  const $$ = (sel, root = document) => Array.from((root || document).querySelectorAll(sel));
  const text = (el) => (el ? (el.innerText || el.textContent || '').trim() : '');
  const norm = (s) => String(s || '')
    .replace(/ /g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  const absUrl = (url) => {
    try { return url ? new URL(url, location.href).href : ''; } catch (e) { return ''; }
  };

  function elementCenter(el) {
    const rect = el.getBoundingClientRect();
    return {
      x: Math.round(rect.left + rect.width / 2),
      y: Math.round(rect.top + rect.height / 2),
    };
  }

  function isVisible(el) {
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return false;
    const style = window.getComputedStyle(el);
    return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
  }

  // ── note modal scoping (the core fix for content extraction) ─
  function getNoteOverlay() {
    const overlay = $('.note-detail-mask, .note-overlay, .note-detail-modal, #noteContainer');
    return overlay && overlay.offsetHeight > 0 ? overlay : null;
  }

  function getNoteRoot() {
    const overlay = getNoteOverlay();
    if (overlay) return overlay;
    for (const sel of ['#noteContainer', '.note-detail-mask', '.note-detail-modal', '.note-detail', '.note-scroller', '.note-content']) {
      const el = $(sel);
      if (isVisible(el)) return el;
    }
    return document;
  }

  const COMMENT_AREA_SELECTOR =
    '.comments-container, .comment-list, .comment-item, .comment-inner, .comment-wrapper, ' +
    '.parent-comment, .reply-item, .sub-comment-item, .child-comment-item, .reply-comment-item, ' +
    '[class*="comment"]';

  function inCommentArea(el) {
    return !!el?.closest?.(COMMENT_AREA_SELECTOR);
  }

  function firstVisibleText(selectors, root, { excludeComments = false } = {}) {
    for (const sel of selectors) {
      for (const el of $$(sel, root)) {
        if (!isVisible(el)) continue;
        if (excludeComments && inCommentArea(el)) continue;
        const value = norm(el.innerText || el.textContent || '');
        if (value) return value;
      }
    }
    return '';
  }

  // ── search input / state / cards ─────────────────────────────
  // 2026-05: the homepage search widget switched from <input> to a
  // <textarea class="textarea"> living inside #search-input-in-feeds
  // (a chat-style composer with an AI helper "问点点" below). The
  // legacy <input> selectors are kept as fallback in case XHS rolls
  // the old UI back to some users.
  const SEARCH_INPUT_SELECTORS = [
    'textarea[placeholder*="搜索"]',
    'input#search-input',
    'input[type="search"]',
    'input[placeholder*="搜索"]',
    '.search-input input',
    '.search-container input',
  ];

  function findSearchInput() {
    for (const sel of SEARCH_INPUT_SELECTORS) {
      for (const el of $$(sel)) {
        if (!(el instanceof HTMLElement)) continue;
        if (el.getBoundingClientRect().width >= 120) return el;
      }
    }
    return undefined;
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
    return {
      ok: trimmed === targetValue.trim(),
      value: trimmed,
    };
  }

  // Selectors for the search-submit affordance, tried in priority order.
  // The 2026-05 chat composer has explicit class names; legacy UIs used a
  // <form> with type=submit or an .icon-search SVG sibling. We don't try
  // to score arbitrary clickable elements anymore — if none of these
  // match, Rust falls back to pressing Enter, which works in practice.
  const SEARCH_SUBMIT_SELECTORS = [
    '.bottom-box-right-submit-button',
    '.submit-button-wrapper',
    'button[type="submit"]',
    '.search-icon',
    '.search-btn',
    '.icon-search',
  ];

  function searchInput() {
    const input = findSearchInput();
    if (!input) return { ok: false, error: 'search_input_not_found' };
    const root = input.closest(
      'form, header, .search-input, .search-container, .search-bar, .search-box, .wendian-wrapper'
    ) || document;

    let submit = null;
    for (const sel of SEARCH_SUBMIT_SELECTORS) {
      const el = root.querySelector(sel) || document.querySelector(sel);
      if (!el) continue;
      const r = el.getBoundingClientRect();
      if (r.width < 12 || r.height < 12) continue;
      submit = elementCenter(el);
      break;
    }

    return {
      ok: true,
      input: elementCenter(input),
      submit,
    };
  }

  function searchState() {
    const cards = $$('section.note-item, [data-note-id], .feeds-page .note-item');
    const input = findSearchInput();
    const url = new URL(location.href);
    const bodyText = text(document.body);
    const tabs = searchTabs();
    const loading = $$('.loading, .spinner, [class*="loading"]').some((el) => isVisible(el));
    const hasNoResults = /暂无|没有找到|无结果|换个词试试|no result/i.test(bodyText);
    return {
      ok: true,
      page_state: url.pathname.includes('/search_result') ? 'search_results' : 'unknown',
      url: location.href,
      url_keyword: url.searchParams.get('keyword') || '',
      input_keyword: input ? String(input.value || input.textContent || '').trim() : '',
      card_count: cards.length,
      tabs,
      active_filter: tabs.find((t) => t.active)?.label || '',
      loading,
      has_no_results: hasNoResults,
      login_required: hasLoginModal(),
    };
  }

  function hasLoginModal() {
    const bodyText = text(document.body);
    const modal = $$('.login-container, .login-modal, .login-box, [class*="login"]').some((el) => {
      if (!isVisible(el)) return false;
      return /手机号登录|扫码|登录后|获取验证码/.test(text(el));
    });
    return modal || /手机号登录[\s\S]{0,80}获取验证码|登录后查看搜索结果|登录后推荐更懂你的笔记/.test(bodyText);
  }

  function pageState() {
    const url = location.href;
    let state = 'unknown';
    if (/xiaohongshu\.com\/user\/profile\//.test(url)) state = 'profile_page';
    else if (/\/(?:explore|discovery|search_result)\/[^/?#]+/.test(url) || getNoteOverlay()) state = 'note_detail';
    else if (url.includes('/search_result')) state = 'search_results';
    else if (/xiaohongshu\.com/.test(url)) state = 'homepage';
    return {
      ok: true,
      state,
      url,
      title: document.title,
      note_open: noteOpen(),
      search: searchState(),
      login_required: hasLoginModal(),
    };
  }

  function searchCards() {
    const fromState = [];
    try {
      const feeds = window.__INITIAL_STATE__?.search?.feeds?._value || [];
      for (let i = 0; i < feeds.length; i++) {
        const item = feeds[i] || {};
        const card = item.noteCard || item.note_card || null;
        if (!card) continue;
        const id = item.id || card.id || card.noteId || '';
        const token = item.xsecToken || item.xsec_token || '';
        fromState.push({
          note_id: id,
          title: card.displayTitle || card.title || '',
          author: card.user?.nickname || card.user?.nickName || '',
          author_id: card.user?.userId || card.user?.id || '',
          author_url: card.user?.userId ? `https://www.xiaohongshu.com/user/profile/${card.user.userId}` : '',
          likes: String(card.interactInfo?.likedCount || card.interactInfo?.likes || ''),
          cover_url: card.cover?.urlDefault || card.cover?.urlPre || '',
          type: card.type || '',
          position: i,
          xsec_token: token,
          link: id && token
            ? `https://www.xiaohongshu.com/explore/${id}?xsec_token=${encodeURIComponent(token)}&xsec_source=pc_search`
            : (id ? `https://www.xiaohongshu.com/explore/${id}` : ''),
        });
      }
    } catch (e) {}
    if (fromState.length) return fromState;

    const cards = $$('section.note-item, [data-note-id], .feeds-page .note-item');
    return cards.map((card, i) => {
      const linkEl = card.querySelector('a[href*="/explore/"], a[href*="/search_result/"]') || card.closest('a') || card.querySelector('a');
      const link = linkEl ? linkEl.href : '';
      const idMatch = link.match(/\/(?:explore|search_result|discovery)\/([^/?#]+)/);
      const noteId = card.dataset?.noteId || (idMatch ? idMatch[1] : '');
      const authorEl = card.querySelector('a[href*="/user/profile/"], .author-wrapper a, .author a');
      const authorUrl = authorEl ? absUrl(authorEl.href || authorEl.getAttribute('href')) : '';
      const authorIdMatch = authorUrl.match(/\/user\/profile\/([^/?#]+)/);
      return {
        note_id: noteId,
        title: text(card.querySelector('.title, .note-title, a.title span')),
        author: text(card.querySelector('.author-wrapper .name, .author .name, .nick-name')),
        author_id: authorIdMatch ? authorIdMatch[1] : '',
        author_url: authorUrl,
        likes: text(card.querySelector('.like-wrapper .count, .engagement .like .count, .count')),
        cover_url: card.querySelector('.cover img, .note-cover img, img')?.src || '',
        type: card.querySelector('video, .play-icon, .video-icon, svg[class*="video"], .duration') ? 'video' : 'image',
        position: i,
        xsec_token: '',
        link,
      };
    }).filter((c) => c.note_id || c.title || c.link);
  }

  // ── search tabs (categories: 全部/图文/视频/用户) ─────────────
  const SEARCH_TAB_LABELS = ['全部', '图文', '视频', '用户'];

  function searchTabs() {
    const seen = new Set();
    const out = [];
    for (const el of $$('button, a, div, span')) {
      const label = text(el);
      if (!SEARCH_TAB_LABELS.includes(label) || seen.has(label)) continue;
      if (!(el instanceof HTMLElement)) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width < 24 || rect.height < 18) continue;
      seen.add(label);
      const cls = el.className || '';
      const active = el.getAttribute('aria-selected') === 'true'
        || /\bactive\b|current|selected/.test(cls)
        || el.getAttribute('data-active') === 'true';
      out.push({ label, active, ...elementCenter(el) });
    }
    return out;
  }

  function clickSearchTab(label) {
    if (!SEARCH_TAB_LABELS.includes(String(label || ''))) {
      return { ok: false, error: `unsupported_tab:${label}` };
    }
    const tab = searchTabs().find((t) => t.label === label);
    if (!tab) return { ok: false, error: 'tab_not_found' };
    return { ok: true, label, x: tab.x, y: tab.y, was_active: tab.active };
  }

  // ── search filter popup (hover-triggered 筛选 panel) ─────────
  // This script only reports the panel as the DOM presents it — each group's
  // visible title plus its visible tags. The canonical key/option vocabulary
  // lives Rust-side (XHS_SEARCH_FILTERS); keeping it out of here avoids two
  // lists drifting apart.

  function findSearchFilterTrigger() {
    // The 筛选 trigger sits in the results header normally, but when the
    // 问点点 AI summary panel shows up for a query the layout shifts and the
    // trigger moves into the AI section (`.filter.ai-chat-filter`). Try the
    // header first, then progressively broaden so both layouts work; in every
    // case require a visible, filter-classed element whose text is 筛选.
    const selectors = [
      '.search-layout__top > .filter, .search-layout__top [class~="filter"]',
      '.search-layout [class~="filter"]',
      '.ai-chat-filter, [class~="filter"]',
    ];
    for (const selector of selectors) {
      for (const el of $$(selector)) {
        if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
        if (text(el).includes('筛选')) return el;
      }
    }
    return null;
  }

  function searchFilterTrigger() {
    const trigger = findSearchFilterTrigger();
    if (!trigger) {
      return { ok: false, error: 'filter_trigger_not_found' };
    }
    const value = text(trigger);
    const label = value.includes('已筛选') ? '已筛选' : '筛选';
    return { ok: true, label, ...elementCenter(trigger) };
  }

  function findSearchFilterPanel() {
    for (const el of $$('.filter-panel, .filter-container')) {
      if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
      const value = text(el);
      if (value.includes('排序依据') && value.includes('发布时间')) {
        return el;
      }
    }
    return null;
  }

  function findFilterOperation(panel, label) {
    for (const el of $$('.operation-container .operation', panel)) {
      if (!(el instanceof HTMLElement) || !isVisible(el)) continue;
      if (text(el) === label) return el;
    }
    return null;
  }

  function searchFilters() {
    const panel = findSearchFilterPanel();
    if (!panel) return { ok: false, error: 'filter_panel_not_found' };

    const groups = [];
    for (const groupEl of $$('.filters-wrapper .filters', panel)) {
      if (!(groupEl instanceof HTMLElement) || !isVisible(groupEl)) continue;
      const title = text($('span', groupEl));
      if (!title) continue;
      const options = [];
      for (const tag of $$('.tag-container .tags', groupEl)) {
        if (!(tag instanceof HTMLElement) || !isVisible(tag)) continue;
        const label = text(tag);
        if (!label) continue;
        options.push({
          label,
          active: /\bactive\b/.test(String(tag.className || '')),
          ...elementCenter(tag),
        });
      }
      if (options.length) {
        groups.push({ title, options });
      }
    }

    const resetEl = findFilterOperation(panel, '重置');
    const closeEl = findFilterOperation(panel, '收起');
    return {
      ok: true,
      groups,
      reset: resetEl ? elementCenter(resetEl) : null,
      close: closeEl ? elementCenter(closeEl) : null,
    };
  }

  // ── card click / note open / note close ──────────────────────
  function findCardElement(arg) {
    const cards = $$('section.note-item, [data-note-id], .feeds-page .note-item');
    if (!cards.length) return null;
    const noteId = String((arg && arg.note_id) || '').trim();
    if (noteId) {
      for (const card of cards) {
        if (card.dataset?.noteId === noteId) return card;
        const link = card.querySelector('a[href*="/explore/"], a[href*="/search_result/"], a[href*="/discovery/"]');
        if (link?.href?.includes(noteId)) return card;
      }
    }
    const index = arg && Number.isInteger(arg.index) ? arg.index : -1;
    return index >= 0 && index < cards.length ? cards[index] : null;
  }

  function clickCard(arg) {
    // Click cover/img — NOT the <a> tag (XHS blocks direct /explore/<id>
    // navigation with a 404). React handler intercepts cover clicks to open
    // the note as an in-page modal.
    const card = findCardElement(arg);
    if (!card) return { ok: false, error: 'card_not_found' };
    card.scrollIntoView({ block: 'center', inline: 'center' });
    const cover = card.querySelector('.cover, .cover-ld, .note-cover, img');
    for (const target of cover ? [cover, card] : [card]) {
      const rect = target.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) continue;
      return {
        ok: true,
        target: target === cover ? 'cover' : 'card',
        ...elementCenter(target),
        note_id: card.dataset?.noteId || '',
      };
    }
    return { ok: false, error: 'card_zero_sized' };
  }

  function closeNote() {
    const selectors = [
      '.close-circle', '.note-detail-mask .close', '.note-overlay .close',
      '.note-modal .close', '.reds-note-detail .close', '.close-button',
      '.close-btn', '.note-close', 'button.close', '.icon-close',
      '[aria-label="关闭"]', 'button[aria-label*="close" i]',
      '.note-detail-mask svg',
    ];
    for (const sel of selectors) {
      const el = $(sel);
      if (!(el instanceof HTMLElement || el instanceof SVGElement)) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        return { ok: true, selector: sel, ...elementCenter(el) };
      }
    }
    return { ok: false, error: 'close_button_not_found' };
  }

  function noteOpen() {
    const url = location.href;
    return {
      ok: true,
      url,
      on_detail_route: /\/(?:explore|discovery|search_result)\/[^/?#]+/.test(url),
      has_modal: !!getNoteOverlay(),
      login_required: hasLoginModal(),
    };
  }

  // ── note content extraction (root-scoped + visible-only) ─────
  function detectNoteType(root) {
    return root?.querySelector?.('video') ? 'video' : 'image';
  }

  function extractNoteIdFromUrl() {
    const m = location.href.match(/\/(?:explore|discovery|search_result)\/([^/?#]+)/);
    return m ? m[1] : '';
  }

  function profileIdFromUrl(url) {
    const m = String(url || location.href).match(/\/user\/profile\/([^/?#]+)/);
    return m ? m[1] : '';
  }

  // URLs that match the fallback selectors but aren't real note carousel
  // images: author/commenter avatars, sponsor icons, sticker assets, etc.
  // Note carousel images come from sns-webpic-* / ci.xiaohongshu.com.
  const NON_NOTE_IMAGE_PATTERNS = [
    /\/avatar\//i,                            // sns-avatar-qc.xhscdn.com/avatar/...
    /\/comment\//i,                           // comment-area image attachments
    /picasso-static\.xiaohongshu\.com/i,      // UI / fe-platform assets
    /fe-static\.xhscdn\.com/i,                // misc static assets
  ];

  function cleanImageUrl(url) {
    const value = absUrl(url || '');
    if (!value || value.startsWith('data:') || value.startsWith('blob:')) return '';
    if (NON_NOTE_IMAGE_PATTERNS.some((re) => re.test(value))) return '';
    return value
      .replace(/^http:\/\//i, 'https://')
      .replace(/imageView2\/\d\/w\/\d+\/format\/[^/?#]+/i, '');
  }

  const NOTE_IMAGE_SELECTORS = [
    '.note-slider img', '.carousel img', '.carousel-image img',
    '.swiper-slide img', '.media-container img', '.note-detail img',
    '#noteContainer img',
  ];

  function collectImageUrls(root) {
    const urls = [];
    const seen = new Set();
    for (const sel of NOTE_IMAGE_SELECTORS) {
      for (const img of $$(sel, root)) {
        if (!isVisible(img)) continue;
        // Broad fallback selectors (#noteContainer img, .note-detail img)
        // also match imgs inside the comment area. Skip them — note carousel
        // imgs live above the comments DOM.
        if (inCommentArea(img)) continue;
        const candidates = [
          img.currentSrc, img.src, img.getAttribute('src'),
          img.getAttribute('data-src'), img.getAttribute('data-original'),
        ];
        for (const candidate of candidates) {
          const url = cleanImageUrl(candidate);
          if (!url || seen.has(url)) continue;
          seen.add(url);
          urls.push(url);
        }
      }
    }
    return urls;
  }

  function cleanLocationText(value) {
    const cleaned = norm(value);
    if (!cleaned) return '';
    const lines = cleaned.split(/\n+/).map(norm).filter(Boolean);
    if (!lines.length) return '';
    // iPhone Live Photo badges render as visible overlay text in the media
    // area; they are not note locations/POIs.
    if (lines.every((line) => /^live$/i.test(line))) return '';
    if (/^live(?:\s+live)+$/i.test(lines.join(' '))) return '';
    // Real note locations are compact labels like "北京"; multi-line blobs
    // here are almost always media overlay or layout noise.
    if (lines.length > 1 || cleaned.length > 40) return '';
    return cleaned;
  }

  function collectVideoInfo(root) {
    const video = root.querySelector?.('video');
    const candidates = [];
    const push = (url, source) => {
      const value = absUrl(url || '');
      if (!value || candidates.some((item) => item.url === value)) return;
      candidates.push({ url: value, source });
    };
    if (video) {
      push(video.currentSrc, 'video.currentSrc');
      push(video.src, 'video.src');
      for (const sourceEl of $$('source', video)) push(sourceEl.src || sourceEl.getAttribute('src'), 'source');
    }
    try {
      for (const entry of performance.getEntriesByType('resource')) {
        const name = String(entry.name || '');
        if (/(\.mp4|\.m3u8|\.m4v|\.mov)(\?|$)|video|vod|hls|sns-video/i.test(name)) {
          push(name, 'performance');
        }
      }
    } catch (e) {}
    const poster = video?.poster || root.querySelector?.('img')?.src || '';
    return {
      url: candidates[0]?.url || '',
      resolved_url: candidates.find((item) => /^https?:/.test(item.url) && !item.url.startsWith('blob:'))?.url || candidates[0]?.url || '',
      poster_url: cleanImageUrl(poster),
      duration_s: Number.isFinite(video?.duration) ? video.duration : null,
      source_urls: candidates.map((item) => item.url),
      candidates,
    };
  }

  // Stop / ignore line filters for the root-text fallback. Trimmed from
  // flowlens to the cases that actually fire during normal note reads.
  const STOP_LINE = /^(?:共\s*\d*\s*条评论|展开|收起|说点什么|猜你想搜)$|^(?:刚刚|\d+\s*(?:秒|分钟|小时|天)前|昨天|前天)$|^\d{1,2}-\d{1,2}(?:\s+\S+)?$|^\d{4}-\d{1,2}-\d{1,2}/;
  const IGNORE_LINE = /^(?:已关注|关注|作者|赞|收藏|评论|分享)$/;

  function extractContentFromRootText(root, title, author) {
    const lines = norm(text(root)).split(/\n+/).map(norm).filter(Boolean);
    if (!lines.length) return '';
    let start = -1;
    if (title) start = lines.findIndex((line) => line === title || line.includes(title) || title.includes(line));
    if (start < 0 && author) {
      const i = lines.findIndex((line) => line === author);
      if (i >= 0) start = i;
    }
    if (start < 0) return '';
    const body = [];
    for (const line of lines.slice(start + 1)) {
      if (!line || line === title || line === author) continue;
      if (STOP_LINE.test(line)) break;
      if (IGNORE_LINE.test(line)) continue;
      body.push(line);
    }
    const cleaned = norm(body.join('\n'));
    return cleaned.length >= 6 ? cleaned : '';
  }

  function note() {
    const root = getNoteRoot();
    const title = firstVisibleText(
      ['#detail-title', '.note-content .title', '.note-scroller .title', '.note-detail .title', 'h1'],
      root, { excludeComments: true },
    );
    const author = firstVisibleText(
      ['.author-container .username', '.author-wrapper .username', '.info .username', '.user-name'],
      root,
    );
    const authorLink = root.querySelector?.('a[href*="/user/profile/"], .author-container a, .author-wrapper a');
    const authorUrl = authorLink ? absUrl(authorLink.href || authorLink.getAttribute('href')) : '';
    const contentSelectors = [
      '#detail-desc .note-text', '#detail-desc',
      '.note-content #detail-desc', '.note-scroller #detail-desc',
      '.note-content .note-text', '.note-scroller .note-text',
      '.note-content .desc', '.note-scroller .desc', '.note-detail .desc',
    ];
    let content = firstVisibleText(contentSelectors, root, { excludeComments: true });
    let contentSource = content ? 'selector' : '';
    if (!content) {
      content = extractContentFromRootText(root, title, author);
      if (content) contentSource = 'root_text';
    }
    const likes = firstVisibleText(
      ['.like-wrapper .count', '.engage-bar .like .count', '[data-type="like"] .count'],
      root, { excludeComments: true },
    );
    const favorites = firstVisibleText(
      ['.collect-wrapper .count', '.engage-bar .collect .count', '[data-type="collect"] .count'],
      root, { excludeComments: true },
    );
    const commentsCount = firstVisibleText(
      ['.chat-wrapper .count', '.engage-bar .chat .count', '[data-type="chat"] .count'],
      root, { excludeComments: true },
    );
    const shares = firstVisibleText(
      ['.share-wrapper .count', '.engage-bar .share .count', '[data-type="share"] .count'],
      root, { excludeComments: true },
    );
    const hashtags = $$('.hash-tag a, a[href*="/page/topics/"], #detail-desc a.tag', root)
      .filter(isVisible).map(text).filter(Boolean);
    const rootText = norm(text(root));
    const date = (rootText.match(/\b\d{4}-\d{1,2}-\d{1,2}\b|\b\d{1,2}-\d{1,2}\b/) || [''])[0];
    const ipLocation = (rootText.match(/IP属地[:：]?\s*([\u4e00-\u9fffA-Za-z0-9_-]+)/) || [])[1] || '';
    const locationText = cleanLocationText(
      firstVisibleText(['.location, .poi, [class*="location"], [class*="poi"]'], root, { excludeComments: true })
    );
    const type = detectNoteType(root);
    const imageUrls = collectImageUrls(root);
    const video = type === 'video' ? collectVideoInfo(root) : null;
    return {
      note_id: extractNoteIdFromUrl(),
      url: location.href,
      type,
      title,
      author,
      author_id: profileIdFromUrl(authorUrl),
      author_url: authorUrl,
      content,
      content_source: contentSource,
      date,
      location: locationText,
      ip_location: ipLocation,
      likes,
      favorites,
      comments_count: commentsCount,
      shares,
      hashtags,
      image_count: imageUrls.length,
      image_urls: imageUrls,
      video,
    };
  }

  function carouselImages(opts = {}) {
    const root = getNoteRoot();
    const urls = collectImageUrls(root);
    const max = Number(opts.max_images) || 12;
    return {
      ok: true,
      image_urls: urls.slice(0, max),
      image_count: urls.length,
    };
  }

  function profileInfo() {
    const displayName = firstVisibleText(
      ['.user-name', '.profile-name', '.nickname', '.name', 'h1'],
      document,
    );
    const bio = firstVisibleText(['.user-desc', '.profile-desc', '.desc', '.bio'], document);
    const body = norm(text(document.body));
    const xhsId = (body.match(/小红书号[:：]?\s*([A-Za-z0-9_.-]+)/) || [])[1] || profileIdFromUrl();
    const statText = (label) => {
      const re = new RegExp(`([0-9.,万wWkK+]+)\\s*(?:${label})`);
      return (body.match(re) || [])[1] || '';
    };
    return {
      ok: true,
      display_name: displayName,
      xhs_id: xhsId,
      profile_url: location.href,
      bio,
      followers: statText('粉丝'),
      following: statText('关注'),
      likes_and_collections: statText('获赞与收藏|获赞|赞与收藏'),
    };
  }

  function profileCards() {
    return searchCards();
  }

  // ── hydration wait — single round-trip Promise loop ──────────
  function countLoadingIndicators(root) {
    return $$(
      '.loading, [class*="loading"], [class*="Loading"], [class*="skeleton"], [class*="Skeleton"], [class*="shimmer"]',
      root,
    ).filter(isVisible).length;
  }

  function pendingHydration(root) {
    const preview = norm(text(root)).slice(0, 1200);
    if (/(^|\n)加载中(?:\n|$)/.test(preview)) return true;
    if (/正在加载|请稍候|loading/i.test(preview)) return true;
    return countLoadingIndicators(root) > 0;
  }

  function noteWithWait(opts = {}) {
    const timeoutMs = Math.max(500, Number(opts.timeout_ms) || 8000);
    const shellSettleMs = Math.max(500, Number(opts.shell_settle_ms) || 3500);
    return new Promise((resolve) => {
      const startedAt = Date.now();
      let shellSeenAt = 0;
      let best = null;
      let attempts = 0;
      const tick = () => {
        attempts += 1;
        const root = getNoteRoot();
        const value = note();
        const hasContent = !!norm(value.content);
        const hasShell = !!(value.note_id || value.title || value.author || value.likes || value.comments_count);
        const pending = pendingHydration(root);
        if (hasShell) { best = value; if (!shellSeenAt) shellSeenAt = Date.now(); }
        if (hasContent) {
          resolve({ ready: true, reason: 'content_ready', waited_ms: Date.now() - startedAt, attempts, note: value });
          return;
        }
        if (hasShell && !pending && Date.now() - shellSeenAt >= shellSettleMs) {
          resolve({ ready: true, reason: 'shell_settled', waited_ms: Date.now() - startedAt, attempts, note: value });
          return;
        }
        if (Date.now() - startedAt >= timeoutMs) {
          resolve({ ready: !!best, reason: best ? 'timeout_with_shell' : 'timeout', waited_ms: Date.now() - startedAt, attempts, note: best || value });
          return;
        }
        setTimeout(tick, 250);
      };
      tick();
    });
  }

  // ── comments ─────────────────────────────────────────────────
  const COMMENT_ROOT_SELECTOR = '.comment-item, .parent-comment, .comment-inner, .comments-container .comment-item-inner, .comment-wrapper';
  const SUB_COMMENT_SELECTOR = '.reply-item, .sub-comment-item, .child-comment-item, .reply-comment-item';

  function parseCount(raw) {
    const v = String(raw || '').trim().toLowerCase().replace(/,/g, '').replace(/\+/g, '');
    const m = v.match(/(\d+(?:\.\d+)?)(万|w|k)?/);
    if (!m) return 0;
    let n = parseFloat(m[1]);
    if (m[2] === '万' || m[2] === 'w') n *= 10000;
    else if (m[2] === 'k') n *= 1000;
    return Math.round(n);
  }

  function firstText(selectors, root) {
    for (const sel of selectors) {
      const el = root.querySelector?.(sel);
      const v = text(el);
      if (v) return v;
    }
    return '';
  }

  function parseComment(item, includeChildren) {
    const username = firstText(['.name', '.user-name', '.nickname', '.author-name'], item);
    const content = firstText(['.content', '.comment-text', '.note-text', '.desc', '[class*="content"]'], item);
    const likes = firstText(['.like .count', '.like-wrapper .count', '.interact-wrapper .count', '[class*="like"] .count'], item);
    const time = firstText(['.time', '.date', '.create-time', '.comment-time', '[class*="time"]'], item);
    const badge = firstText(['.author-tag', '.tag.author', '.reply-tag', '.user-tag', '[class*="author-tag"]'], item);
    const top = firstText(['.top-tag', '.pinned-tag', '[class*="top-tag"]'], item);
    const subs = [];
    if (includeChildren) {
      const children = $$(SUB_COMMENT_SELECTOR, item).filter((sub) => !sub.parentElement?.closest(SUB_COMMENT_SELECTOR));
      for (const child of children) {
        const parsed = parseComment(child, false);
        if (parsed.text) subs.push(parsed);
      }
    }
    return {
      username,
      text: content,
      likes,
      like_count: parseCount(likes),
      time,
      is_author_reply: /作者|博主|楼主/.test(badge),
      is_pinned: /置顶/.test(top),
      reply_count: subs.length,
      sub_comments: subs,
    };
  }

  function comments(opts = {}) {
    const root = getNoteRoot();
    const items = $$(COMMENT_ROOT_SELECTOR, root)
      .filter((item) => !item.parentElement?.closest(COMMENT_ROOT_SELECTOR));
    const seen = new Set();
    let out = [];
    for (const item of items) {
      const parsed = parseComment(item, true);
      if (!parsed.text) continue;
      const key = `${parsed.username}:${parsed.text.slice(0, 30)}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(parsed);
    }
    if (opts.prefer_hot !== false) {
      out.sort((a, b) => (b.like_count + b.reply_count * 3 + (b.is_pinned ? 10 : 0))
                       - (a.like_count + a.reply_count * 3 + (a.is_pinned ? 10 : 0)));
    }
    const max = Number(opts.max_comments) || 0;
    if (max > 0) out = out.slice(0, max);
    return out;
  }

  function commentsSignature(items) {
    return items
      .map((item) => `${item.username || ''}:${String(item.text || '').slice(0, 40)}`)
      .join('|');
  }

  function commentsWithWait(opts = {}) {
    const timeoutMs = Math.max(500, Number(opts.timeout_ms) || 5000);
    const settleMs = Math.max(300, Number(opts.settle_ms) || 900);
    const emptySettleMs = Math.max(700, Number(opts.empty_settle_ms) || 1800);
    return new Promise((resolve) => {
      const startedAt = Date.now();
      let latest = [];
      let lastSig = '';
      let stableSince = startedAt;
      let emptyShellSeenAt = 0;
      let attempts = 0;
      const tick = () => {
        attempts += 1;
        const root = getNoteRoot();
        const items = comments(opts);
        const sig = commentsSignature(items);
        if (sig !== lastSig) {
          lastSig = sig;
          stableSince = Date.now();
        }
        latest = items;

        if (items.length > 0 && Date.now() - stableSince >= settleMs) {
          resolve({ ready: true, reason: 'comments_ready', waited_ms: Date.now() - startedAt, attempts, comments: items });
          return;
        }

        const noCommentCopy = /这是一片荒地|还没有评论哦|暂无评论|还没有评论|抢首评/.test(norm(text(root)).slice(0, 1200));
        if (!items.length && noCommentCopy && !pendingHydration(root)) {
          if (!emptyShellSeenAt) emptyShellSeenAt = Date.now();
          if (Date.now() - emptyShellSeenAt >= emptySettleMs) {
            resolve({ ready: true, reason: 'no_comments', waited_ms: Date.now() - startedAt, attempts, comments: [] });
            return;
          }
        } else {
          emptyShellSeenAt = 0;
        }

        if (Date.now() - startedAt >= timeoutMs) {
          resolve({ ready: items.length > 0, reason: items.length > 0 ? 'timeout_with_comments' : 'timeout', waited_ms: Date.now() - startedAt, attempts, comments: latest });
          return;
        }
        setTimeout(tick, 250);
      };
      tick();
    });
  }

  // ── search/feed scroll ───────────────────────────────────────
  // Window-level scroll used to lazy-load more search cards. Default jumps to
  // the current document bottom (window-size independent, no hard-coded pixel
  // step) so the site fetches the next page. With `nudge_up`, instead scrolls
  // back up ~1/10 of a screen: XHS sometimes ignores a too-fast jump to the
  // bottom and won't load more, but a small reverse scroll reliably jogs its
  // infinite-scroll observer. The caller waits for new cards by polling
  // searchCards.
  function scrollFeed(opts = {}) {
    const el = document.scrollingElement || document.documentElement;
    const before = el.scrollTop;
    const beforeHeight = el.scrollHeight;
    if (opts && opts.nudge_up) {
      const step = Math.max(80, Math.round(window.innerHeight / 10));
      window.scrollBy({ top: -step, left: 0, behavior: 'instant' });
    } else {
      window.scrollTo({ top: el.scrollHeight, left: 0, behavior: 'instant' });
    }
    const after = el.scrollTop;
    return {
      ok: true,
      before,
      after,
      moved: after !== before,
      scroll_height: beforeHeight,
      inner_height: window.innerHeight,
    };
  }

  // ── modal-internal scroll (Promise-resolved) ─────────────────
  function scrollInNote(opts = {}) {
    const pixels = Number(opts.pixels) || 400;
    return new Promise((resolve) => {
      function scrollable(el) {
        if (!(el instanceof HTMLElement)) return false;
        const style = window.getComputedStyle(el);
        const overflow = style.overflowY || style.overflow || '';
        return el.scrollHeight > el.clientHeight + 24 && ['auto', 'scroll', 'overlay'].includes(overflow);
      }
      const overlay = $('.note-detail-mask, .note-overlay, .note-detail-modal, .note-detail, #noteContainer');
      const candidates = [
        ...$$([
          '.note-scroller', '.note-content', '.note-detail .content', '.scroll-container',
          '.note-detail', '#noteContainer',
          '.note-detail-mask [class*="scroll"]', '.note-detail-mask [class*="content"]',
        ].join(', ')),
        overlay,
      ].filter(Boolean);
      const seen = new Set();
      const unique = [];
      for (const node of candidates) {
        if (!(node instanceof HTMLElement) || seen.has(node)) continue;
        seen.add(node);
        unique.push(node);
      }
      unique.sort((a, b) => (b.scrollHeight - b.clientHeight) - (a.scrollHeight - a.clientHeight));
      const container = unique.find(scrollable) || null;
      if (container) {
        const before = container.scrollTop;
        container.scrollBy({ top: pixels, behavior: 'smooth' });
        setTimeout(() => {
          const after = container.scrollTop;
          resolve({
            ok: after !== before,
            container: container.className || container.id || container.tagName,
            delta: after - before,
            error: after !== before ? '' : 'scroll_did_not_move',
          });
        }, 900);
      } else {
        const before = window.scrollY;
        window.scrollBy({ top: pixels, behavior: 'smooth' });
        setTimeout(() => {
          const after = window.scrollY;
          resolve({
            ok: after !== before,
            container: 'window',
            delta: after - before,
            error: after !== before ? '' : 'scroll_did_not_move',
          });
        }, 900);
      }
    });
  }

  return {
    note,
    noteWithWait,
    pageState,
    searchCards,
    searchInput,
    setSearchInput,
    searchState,
    searchTabs,
    clickSearchTab,
    searchFilterTrigger,
    searchFilters,
    clickCard,
    closeNote,
    noteOpen,
    comments,
    commentsWithWait,
    scrollFeed,
    scrollInNote,
    carouselImages,
    profileInfo,
    profileCards,
  };
})();
