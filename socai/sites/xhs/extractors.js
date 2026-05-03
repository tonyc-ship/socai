const SocaiXhsExtractors = (() => {
  const searchInputSelectors = [
    'input#search-input',
    'input[type="search"]',
    'input[placeholder*="搜索"]',
    '.search-input input',
    '.search-container input',
  ];

  const text = (el) => (el ? (el.innerText || el.textContent || '').trim() : '');
  const norm = (s) => String(s || '').replace(/\s+\n/g, '\n').replace(/\n\s+/g, '\n').trim();

  function findSearchInput() {
    return searchInputSelectors
      .map((selector) => document.querySelector(selector))
      .find((el) => el instanceof HTMLElement && el.getBoundingClientRect().width >= 120);
  }

  function searchInput() {
    const input = findSearchInput();
    if (!input) return { ok: false, error: 'search_input_not_found' };

    const inputRect = input.getBoundingClientRect();
    const root = input.closest('form, header, .search-input, .search-container, .search-bar, .search-box') || document;
    const inputCenterY = inputRect.top + inputRect.height / 2;
    const rawSubmitCandidates = [
      ...root.querySelectorAll('button, [role="button"], a, div, span, svg, .search-icon, .search-btn, .icon-search'),
      ...document.querySelectorAll('button, [role="button"], a, div, span, svg, .search-icon, .search-btn, .icon-search'),
    ];
    const submitCandidates = [...new Set(rawSubmitCandidates)]
      .filter((el) => el instanceof HTMLElement || el instanceof SVGElement)
      .map((el) => {
        const clickable = el.closest?.('button, [role="button"], a, div, span') || el;
        const rect = clickable.getBoundingClientRect();
        const meta = [
          clickable.getAttribute?.('aria-label') || '',
          clickable.getAttribute?.('title') || '',
          clickable.className || '',
          el.getAttribute?.('aria-label') || '',
          el.getAttribute?.('title') || '',
          el.className || '',
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
        rect.width >= 12
        && rect.height >= 12
        && rect.right >= inputRect.left
        && rect.left <= inputRect.right + 180
        && score > -140
      ))
      .sort((a, b) => b.score - a.score);

    const submit = submitCandidates[0] || null;
    return {
      ok: true,
      input: {
        x: Math.round(inputRect.left + inputRect.width / 2),
        y: Math.round(inputRect.top + inputRect.height / 2),
      },
      submit: submit ? {
        x: Math.round(submit.rect.left + submit.rect.width / 2),
        y: Math.round(submit.rect.top + submit.rect.height / 2),
      } : null,
    };
  }

  function searchState() {
    const cards = Array.from(document.querySelectorAll('section.note-item, [data-note-id], .feeds-page .note-item'));
    const input = findSearchInput();
    const url = new URL(location.href);
    const bodyText = text(document.body);
    const tabs = Array.from(document.querySelectorAll('.search-tab, .tab, [role="tab"], .search-tabs span, .search-tabs a'))
      .map((el) => text(el))
      .filter(Boolean);
    const isVisible = (el) => {
      if (!el) return false;
      const style = window.getComputedStyle(el);
      return style.display !== 'none'
        && style.visibility !== 'hidden'
        && Number(style.opacity || 1) > 0
        && el.getClientRects().length > 0;
    };
    const loading = Array.from(document.querySelectorAll('.loading, .spinner, [class*="loading"]'))
      .some((el) => isVisible(el));
    const hasNoResults = /暂无|没有找到|无结果|no result/i.test(bodyText);
    const urlKeyword = url.searchParams.get('keyword') || '';
    const inputKeyword = input ? String(input.value || input.textContent || '').trim() : '';
    const pageState = url.pathname.includes('/search_result') ? 'search_results' : 'unknown';

    return {
      ok: true,
      page_state: pageState,
      url: location.href,
      url_keyword: urlKeyword,
      input_keyword: inputKeyword,
      card_count: cards.length,
      tabs,
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

    const cards = Array.from(document.querySelectorAll('section.note-item, [data-note-id], .feeds-page .note-item'));
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
    }).filter((card) => card.note_id || card.title || card.link);
  }

  function note() {
    const scopedText = (el) => norm(el ? (el.innerText || el.textContent || '') : '');
    const first = (selectors) => {
      for (const sel of selectors) {
        const el = document.querySelector(sel);
        const value = scopedText(el);
        if (value) return value;
      }
      return '';
    };
    const url = location.href;
    const idMatch = url.match(/\/(?:explore|search_result|discovery)\/([^/?#]+)/);
    const raw = norm(document.body ? document.body.innerText || '' : '');
    const title = first(['#detail-title', '.note-content .title', '.note-scroller .title', '.note-detail .title', 'h1']);
    const author = first(['.author-container .username', '.author-wrapper .username', '.info .username', '.user-name']);
    const content = first([
      '#detail-desc .note-text',
      '#detail-desc',
      '.note-content .note-text',
      '.note-scroller .note-text',
      '.note-content .desc',
      '.note-scroller .desc',
    ]);
    const hashtags = Array.from(document.querySelectorAll('.hash-tag a, a[href*="/page/topics/"], #detail-desc a.tag'))
      .map((el) => scopedText(el)).filter(Boolean);
    return {
      note_id: idMatch ? idMatch[1] : '',
      url,
      title,
      author,
      content: content || raw.slice(0, 2000),
      hashtags,
      likes: first(['.like-wrapper .count', '.engage-bar .like .count', '[data-type="like"] .count']),
      favorites: first(['.collect-wrapper .count', '.engage-bar .collect .count', '[data-type="collect"] .count']),
      comments_count: first(['.chat-wrapper .count', '.engage-bar .chat .count', '[data-type="chat"] .count']),
      raw_text_excerpt: raw.slice(0, 2000),
    };
  }

  return {
    note,
    searchCards,
    searchInput,
    searchState,
  };
})();
