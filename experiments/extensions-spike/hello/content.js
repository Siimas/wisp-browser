const b = document.createElement("div");
b.id = "wisp-ext-hello";
b.style.cssText = "position:fixed;left:16px;bottom:16px;max-width:900px;padding:12px 16px;background:#5b3df5;color:#fff;font:600 14px system-ui;border-radius:10px;z-index:99999";
b.textContent = "content script ran…";
document.body.appendChild(b);
const ask = k => chrome.runtime.sendMessage({ kind: k }).then(r => k + "=" + JSON.stringify(r), e => k + " ERR " + e);
Promise.all(["sync", "async", "promise"].map(ask)).then(rs => { b.textContent = "content script ran · " + rs.join(" · "); });
