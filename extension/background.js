'use strict';
// To Picture — отправляет ссылку на видео в программу (через native messaging) вместе с cookies сайта:
// если сайт показывает видео только после входа, программа скачает его так же, как его видите вы.

const HOST = 'com.topicture.app';
const msg = (key) => chrome.i18n.getMessage(key) || key;

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: 'link', title: msg('menuLink'), contexts: ['link'] });
    chrome.contextMenus.create({ id: 'media', title: msg('menuMedia'), contexts: ['video', 'audio'] });
    chrome.contextMenus.create({ id: 'page', title: msg('menuPage'), contexts: ['page', 'selection', 'image'] });
  });
});

chrome.action.onClicked.addListener((tab) => send(tab.url, tab));

chrome.contextMenus.onClicked.addListener((info, tab) => {
  let url = info.pageUrl;
  if (info.menuItemId === 'link') url = info.linkUrl;
  // у <video> адрес обычно blob: или поток без звука — отправляем страницу (или встроенный плеер)
  else if (info.menuItemId === 'media') url = (info.frameUrl && info.frameUrl !== info.pageUrl) ? info.frameUrl : info.pageUrl;
  send(url, tab);
});

// домен для cookies: youtu.be — это youtube.com, www. и m. не важны
function siteOf(url) {
  try {
    let h = new URL(url).hostname.toLowerCase().replace(/^(www|m)\./, '');
    if (h === 'youtu.be') h = 'youtube.com';
    return h;
  } catch { return ''; }
}

async function cookiesFor(urls) {
  const seen = new Set(), out = [];
  for (const site of new Set(urls.map(siteOf).filter(Boolean))) {
    let list = [];
    try { list = await chrome.cookies.getAll({ domain: site }); } catch {}
    for (const c of list) {
      const id = `${c.domain}|${c.path}|${c.name}`;
      if (seen.has(id)) continue;
      seen.add(id);
      out.push({ domain: c.domain, hostOnly: c.hostOnly, path: c.path, secure: c.secure, httpOnly: c.httpOnly,
                 expirationDate: c.expirationDate || 0, name: c.name, value: c.value });
    }
  }
  return out;
}

function flash(tab, kind, text) {
  const tabId = tab && tab.id >= 0 ? tab.id : undefined;
  const badge = { ok: '✓', err: '!', bad: '–' }[kind];
  const color = kind === 'ok' ? '#2E7D32' : '#B3261E';
  chrome.action.setBadgeBackgroundColor({ tabId, color });
  chrome.action.setBadgeText({ tabId, text: badge });
  chrome.action.setTitle({ tabId, title: text || msg('actionTitle') });
  setTimeout(() => {
    chrome.action.setBadgeText({ tabId, text: '' });
    chrome.action.setTitle({ tabId, title: msg('actionTitle') });
  }, kind === 'ok' ? 2500 : 6000);
}

async function send(url, tab) {
  if (!/^https?:\/\//i.test(url || '')) return flash(tab, 'bad', msg('notVideoPage'));
  const cookies = await cookiesFor([url, tab && tab.url].filter(Boolean));
  const message = { type: 'add', url, title: (tab && tab.url === url && tab.title) || '', cookies };
  chrome.runtime.sendNativeMessage(HOST, message, (resp) => {
    const err = chrome.runtime.lastError;
    if (err || !resp || !resp.ok) {
      flash(tab, 'err', msg('errNoApp'));
      // программа не установлена или связь не настроена — показываем, что сделать
      if (err) chrome.tabs.create({ url: chrome.runtime.getURL('help.html') });
      return;
    }
    flash(tab, 'ok', msg(resp.launched ? 'sentLaunched' : 'sent'));
  });
}
