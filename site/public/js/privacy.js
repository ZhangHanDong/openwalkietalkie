/* An informational notice: there are no optional analytics/advertising
   cookies to accept. Acknowledgement is saved locally for 180 days. */
(function () {
  'use strict';
  var KEY = 'owt-privacy-notice';
  var VERSION = 1;
  var DURATION = 180 * 24 * 60 * 60 * 1000;
  var acknowledged = false;
  try {
    var saved = JSON.parse(localStorage.getItem(KEY));
    acknowledged = !!saved && saved.version === VERSION && typeof saved.expires === 'number' && saved.expires > Date.now();
  } catch (error) { /* A blocked or invalid store must not break the page. */ }
  var notice = document.createElement('section');
  notice.className = 'privacy-notice';
  notice.setAttribute('aria-label', 'Privacy notice');
  notice.hidden = acknowledged;
  notice.innerHTML = '<div><h2>Your privacy, in plain words</h2>' +
    '<p>We remember your day/night theme on this browser. This site has no advertising or analytics cookies. Cloudflare may use security cookies to protect it.</p>' +
    '<p><a href="/privacy/">Privacy Policy</a> <span aria-hidden="true">·</span> <a href="/cookies/">Cookie information</a></p></div>' +
    '<button class="btn btn--primary" type="button">Got it</button>';
  document.body.appendChild(notice);
  var opener = null;
  notice.querySelector('button').addEventListener('click', function () {
    try { localStorage.setItem(KEY, JSON.stringify({ version: VERSION, expires: Date.now() + DURATION })); } catch (error) {}
    notice.hidden = true;
    if (opener) opener.focus();
  });
  document.querySelectorAll('[data-privacy-open]').forEach(function (link) {
    link.addEventListener('click', function (event) {
      event.preventDefault();
      opener = link;
      notice.hidden = false;
      notice.querySelector('button').focus();
    });
  });
})();
