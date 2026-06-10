(function () {
  function textIncludesAny(text, needles) {
    return needles.some(function (needle) {
      return text.indexOf(needle) !== -1;
    });
  }

  function pageState() {
    var bodyText = (document.body && document.body.innerText) || "";
    var loginPromptVisible = textIncludesAny(bodyText, ["登录后", "登录/注册", "验证码登录"]);
    var profileHintVisible = textIncludesAny(bodyText, ["我的主页", "退出登录", "创作者服务中心"]);
    var signedIn = null;
    if (profileHintVisible) {
      signedIn = true;
    } else if (loginPromptVisible) {
      signedIn = false;
    }

    return {
      site: "dy",
      location: {
        url: location.href,
        title: document.title || "",
        host: location.host,
        path: location.pathname,
        search: location.search,
        hash: location.hash
      },
      ready_state: document.readyState,
      signed_in: signedIn,
      hints: {
        login_prompt_visible: loginPromptVisible,
        profile_hint_visible: profileHintVisible,
        body_text_length: bodyText.length
      },
      viewport: {
        w: innerWidth,
        h: innerHeight,
        sx: scrollX,
        sy: scrollY,
        pw: document.documentElement ? document.documentElement.scrollWidth : 0,
        ph: document.documentElement ? document.documentElement.scrollHeight : 0
      }
    };
  }

  window.SocaiDyPageScripts = {
    pageState: pageState
  };
})();
