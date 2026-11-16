chrome.runtime.onInstalled.addListener(() => chrome.action.setBadgeText({ text: "bg" }));
chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  const info = { tabId: sender.tab && sender.tab.id, tabUrl: sender.tab && sender.tab.url };
  if (msg.kind === "sync") { reply({ sync: "ok", ...info }); return; }
  if (msg.kind === "async") { setTimeout(() => reply({ async: "ok" }), 10); return true; }
  if (msg.kind === "promise") return chrome.storage.local.get({ count: 0 }).then(({ count }) =>
    chrome.storage.local.set({ count: count + 1 }).then(() => ({ promise: "ok", count: count + 1 })));
});
