void function () {
  function text(el) {
    return (el && (el.innerText || el.textContent) || "").replace(/\s+/g, " ").trim();
  }

  function attr(el, name) {
    return (el && el.getAttribute && el.getAttribute(name) || "").trim();
  }

  function visible(el) {
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
  }

  function center(el) {
    const rect = el.getBoundingClientRect();
    return {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2,
      w: rect.width,
      h: rect.height
    };
  }

  function absoluteUrl(url) {
    if (!url) return "";
    try {
      return new URL(url, location.href).href.split("#")[0];
    } catch (_) {
      return "";
    }
  }

  function imageUrl(el) {
    if (!el) return "";
    const img = el.querySelector("img");
    if (img) {
      return img.currentSrc || img.src || attr(img, "data-src") || attr(img, "srcset").split(/\s+/)[0] || "";
    }
    const video = el.querySelector("video");
    if (video) return video.poster || attr(video, "poster") || "";
    const styled = Array.from(el.querySelectorAll("*")).find((node) => {
      const bg = getComputedStyle(node).backgroundImage;
      return bg && bg !== "none" && /url\(/.test(bg);
    });
    if (styled) {
      const match = getComputedStyle(styled).backgroundImage.match(/url\(["']?([^"')]+)["']?\)/);
      return match ? match[1] : "";
    }
    return "";
  }

  function videoIdFromUrl(url) {
    const match = url.match(/\/video\/(\d+)/);
    return match ? match[1] : "";
  }

  function videoIdFromCard(el) {
    if (!el) return "";
    const direct = el.dataset && (el.dataset.awemeId || el.dataset.awemeid);
    if (direct) return direct;
    const attrId = attr(el, "data-aweme-id") || attr(el, "data-awemeid");
    if (attrId) return attrId;
    const id = attr(el, "id");
    const match = id.match(/waterfall_item_(\d+)/);
    return match ? match[1] : "";
  }

  function searchInput() {
    const selectors = [
      "input[type='search']",
      "input[placeholder*='搜索']",
      "input[aria-label*='搜索']",
      "input"
    ];
    for (const selector of selectors) {
      const input = Array.from(document.querySelectorAll(selector)).find(visible);
      if (input) {
        const button = input.closest("form")?.querySelector("button") ||
          input.parentElement?.querySelector("button") ||
          document.querySelector("button[aria-label*='搜索']");
        return {
          ok: true,
          input: center(input),
          button: button && visible(button) ? center(button) : null,
          value: input.value || "",
          placeholder: attr(input, "placeholder"),
          tag: input.tagName.toLowerCase()
        };
      }
    }
    return { ok: false, error: "search_input_not_found" };
  }

  function setSearchInput(arg) {
    const query = String(arg && arg.query || "");
    const loc = searchInput();
    if (!loc.ok) return loc;
    const candidates = [
      "input[type='search']",
      "input[placeholder*='搜索']",
      "input[aria-label*='搜索']",
      "input"
    ];
    const input = candidates.flatMap((selector) => Array.from(document.querySelectorAll(selector))).find(visible);
    if (!input) return { ok: false, error: "search_input_not_found_after_locate" };
    input.focus();
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    if (setter) setter.call(input, "");
    input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward", data: null }));
    if (setter) setter.call(input, query);
    else input.value = query;
    input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: query }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true, value: input.value || query };
  }

  function pageState() {
    const cards = videoCards({ limit: 80 });
    const bodyText = text(document.body).slice(0, 800);
    return {
      ok: true,
      url: location.href,
      title: document.title,
      is_douyin: location.hostname.includes("douyin.com"),
      page_state: location.href.includes("/search") ? "search_results" : "home_or_other",
      card_count: cards.length,
      has_login_copy: /登录|扫码|验证码|验证/.test(bodyText),
      body_text_sample: bodyText
    };
  }

  function searchState() {
    const loc = searchInput();
    const cards = videoCards({ limit: 80 });
    const params = new URLSearchParams(location.search);
    return {
      ok: true,
      url: location.href,
      page_state: location.href.includes("/search") ? "search_results" : "home_or_other",
      input_keyword: loc.ok ? loc.value : "",
      url_keyword: params.get("keyword") || params.get("q") || "",
      card_count: cards.length,
      has_no_results: /没有找到|暂无相关|无搜索结果/.test(text(document.body))
    };
  }

  function nearestCard(anchor) {
    const candidates = [];
    let node = anchor;
    for (let i = 0; node && i < 6; i += 1, node = node.parentElement) {
      const t = text(node);
      const rect = node.getBoundingClientRect();
      if (rect.width > 120 && rect.height > 80 && t.length > 0) candidates.push(node);
    }
    return candidates.find((node) => text(node).length >= text(anchor).length) || anchor;
  }

  function parseMetric(raw, labels) {
    for (const label of labels) {
      const patterns = [
        new RegExp(label + "\\s*[:：]?\\s*([0-9.,万wWkK]+)"),
        new RegExp("([0-9.,万wWkK]+)\\s*" + label)
      ];
      for (const pattern of patterns) {
        const match = raw.match(pattern);
        if (match) return match[1] || "";
      }
    }
    return "";
  }

  function titleFromCard(card, raw) {
    const explicit = Array.from(card.querySelectorAll("[title], [aria-label]"))
      .map((el) => attr(el, "title") || attr(el, "aria-label"))
      .find((value) => value && value.length > 4);
    if (explicit) return explicit;
    const textBlocks = Array.from(card.querySelectorAll("div, span, p"))
      .map(text)
      .filter((value) =>
        value.length >= 8 &&
        !value.includes("@") &&
        !value.includes("为你生成回答") &&
        !/^·?\s*\d/.test(value)
      );
    const longest = textBlocks.sort((a, b) => b.length - a.length)[0];
    if (longest) return longest.slice(0, 180);
    return raw
      .replace(/^\d{1,2}:\d{2}(:\d{2})?/, "")
      .replace(/^[0-9.,万wWkK]+/, "")
      .split(/\s*@/)[0]
      .slice(0, 180)
      .trim();
  }

  function likesFromRaw(raw) {
    const afterDuration = raw.match(/^\d{1,2}:\d{2}(?::\d{2})?\s+([0-9.,万wWkK]+)/);
    if (afterDuration) return afterDuration[1] || "";
    return parseMetric(raw, ["点赞", "赞"]);
  }

  function authorFromRaw(raw) {
    const match = raw.match(/@(.+?)\s*·/);
    return match ? match[1].trim() : "";
  }

  function cardElements() {
    const nodes = [
      ...Array.from(document.querySelectorAll("a[href*='/video/']")).map((anchor) => nearestCard(anchor)),
      ...Array.from(document.querySelectorAll("[data-aweme-id], [data-awemeid], [id^='waterfall_item_']"))
    ];
    const seen = new Set();
    return nodes.filter((node) => {
      if (!node || !visible(node)) return false;
      if (seen.has(node)) return false;
      seen.add(node);
      return true;
    });
  }

  function videoCards(arg) {
    const limit = Math.max(1, Math.min(Number(arg && arg.limit || 80), 300));
    const seen = new Set();
    const cards = [];
    for (const card of cardElements()) {
      const anchor = card.matches("a[href*='/video/']")
        ? card
        : card.querySelector("a[href*='/video/']");
      const anchorUrl = anchor ? absoluteUrl(anchor.href || attr(anchor, "href")) : "";
      const video_id = videoIdFromUrl(anchorUrl) || videoIdFromCard(card);
      if (!video_id || seen.has(video_id)) continue;
      seen.add(video_id);
      const url = anchorUrl || `https://www.douyin.com/video/${video_id}`;
      const raw = text(card);
      const authorLink = Array.from(card.querySelectorAll("a[href*='/user/']")).find(visible);
      const cover_url = absoluteUrl(imageUrl(card));
      if (!cover_url && !anchorUrl) continue;
      const title = titleFromCard(card, raw);
      const likes = likesFromRaw(raw);
      cards.push({
        video_id,
        url,
        title,
        author: authorLink ? text(authorLink) : authorFromRaw(raw),
        author_url: authorLink ? absoluteUrl(authorLink.href || attr(authorLink, "href")) : "",
        cover_url,
        likes,
        comments: parseMetric(raw, ["评论"]),
        shares: parseMetric(raw, ["分享", "转发"]),
        interaction_text: raw.match(/(点赞|评论|分享|收藏)[^。]{0,80}/)?.[0] || "",
        raw_text: raw.slice(0, 500),
        position: cards.length
      });
      if (cards.length >= limit) break;
    }
    return cards;
  }

  function scrollFeed(arg) {
    const pixels = Number(arg && arg.pixels || Math.round(window.innerHeight * 0.85));
    window.scrollBy({ left: 0, top: pixels, behavior: "instant" });
    return { ok: true, x: scrollX, y: scrollY, height: document.documentElement.scrollHeight };
  }

  window.SocaiDouyinPageScripts = {
    pageState,
    searchInput,
    setSearchInput,
    searchState,
    videoCards,
    scrollFeed
  };
}();
