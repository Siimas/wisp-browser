Promise.all([chrome.storage.local.get({ count: 0 }), chrome.tabs.query({ active: true, currentWindow: true })])
  .then(([s, tabs]) => { document.getElementById("out").textContent =
    "storage count = " + s.count + " · active tab = " + (tabs[0] ? tabs[0].title + " (" + tabs[0].url + ")" : "none"); },
    e => { document.getElementById("out").textContent = "ERR " + e; });
