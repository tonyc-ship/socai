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
  const SEARCH_INPUT_SELECTORS = [
    'input#search-input',
    'input[type="search"]',
    'input[placeholder*="搜索"]',
    '.search-input input',
    '.search-container input',
  ];

  function findSearchInput() {
    return SEARCH_INPUT_SELECTORS
      .map((sel) => $(sel))
      .find((el) => el instanceof HTMLElement && el.getBoundingClientRect().width >= 120);
  }

  function searchInput() {
    const input = findSearchInput();
    if (!input) return { ok: false, error: 'search_input_not_found' };
    const inputRect = input.getBoundingClientRect();
    const root = input.closest('form, header, .search-input, .search-container, .search-bar, .search-box') || document;
    const inputCenterY = inputRect.top + inputRect.height / 2;
    const candidates = [
      ...root.querySelectorAll('button, [role="button"], a, div, span, svg, .search-icon, .search-btn, .icon-search'),
      ...document.querySelectorAll('button, [role="button"], a, div, span, svg, .search-icon, .search-btn, .icon-search'),
    ];
    const ranked = [...new Set(candidates)]
      .filter((el) => el instanceof HTMLElement || el instanceof SVGElement)
      .map((el) => {
        const clickable = el.closest?.('button, [role="button"], a, div, span') || el;
        const rect = clickable.getBoundingClientRect();
        const meta = [
          clickable.getAttribute?.('aria-label') || '',
          clickable.getAttribute?.('title') || '',
          clickable.className || '',
        ].join(' ').toLowerCase();
        let score = 0;
        if (/search|搜索|find|query/.test(meta)) score += 100;
        if (/clear|close|cancel|remove|delete|清除|关闭|取消/.test(meta)) score -= 120;
        const centerY = rect.top + rect.height / 2;
        score -= Math.abs(rect.left - inputRect.right);
        score -= Math.abs(centerY - inputCenterY) * 0.6;
        if (rect.left >= inputRect.right - 8) score += 18;
        if (rect.left < inputRect.left - 24) score -= 60;
        if (root.contains(clickable)) score += 18;
        if (rect.left >= inputRect.left && rect.right <= inputRect.right) score -= 20;
        return { rect, score };
      })
      .filter(({ rect, score }) => (
        rect.width >= 12 && rect.height >= 12
        && rect.right >= inputRect.left && rect.left <= inputRect.right + 180
        && score > -140
      ))
      .sort((a, b) => b.score - a.score);
    const submit = ranked[0] || null;
    return {
      ok: true,
      input: { x: Math.round(inputRect.left + inputRect.width / 2), y: Math.round(inputRect.top + inputRect.height / 2) },
      submit: submit ? { x: Math.round(submit.rect.left + submit.rect.width / 2), y: Math.round(submit.rect.top + submit.rect.height / 2) } : null,
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
      return {
        note_id: noteId,
        title: text(card.querySelector('.title, .note-title, a.title span')),
        author: text(card.querySelector('.author-wrapper .name, .author .name, .nick-name')),
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
      out.push({ label, active, x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) });
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
        x: Math.round(rect.left + rect.width / 2),
        y: Math.round(rect.top + rect.height / 2),
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
        return { ok: true, selector: sel, x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
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
    const hashtags = $$('.hash-tag a, a[href*="/page/topics/"], #detail-desc a.tag', root)
      .filter(isVisible).map(text).filter(Boolean);
    return {
      note_id: extractNoteIdFromUrl(),
      url: location.href,
      type: detectNoteType(root),
      title,
      author,
      content,
      content_source: contentSource,
      likes,
      favorites,
      comments_count: commentsCount,
      hashtags,
    };
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
    searchCards,
    searchInput,
    searchState,
    searchTabs,
    clickSearchTab,
    clickCard,
    closeNote,
    noteOpen,
    comments,
    scrollInNote,
  };
})();
