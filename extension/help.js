'use strict';
document.documentElement.lang = chrome.i18n.getUILanguage();
document.querySelectorAll('[data-i18n]').forEach((e) => { e.textContent = chrome.i18n.getMessage(e.dataset.i18n); });
document.title = chrome.i18n.getMessage('helpTitle');
