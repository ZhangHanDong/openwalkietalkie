/* Progressive enhancement. A normal POST has a complete no-JavaScript path. */
(function () {
  'use strict';
  var form = document.getElementById('waitlist-form');
  if (!form) return;
  var status = document.getElementById('waitlist-status');
  var button = form.querySelector('button[type="submit"]');
  var pending = false;
  form.addEventListener('submit', async function (event) {
    event.preventDefault();
    if (pending || !form.reportValidity()) return;
    pending = true;
    button.disabled = true;
    form.setAttribute('aria-busy', 'true');
    button.textContent = 'Joining…';
    status.dataset.state = 'pending';
    status.textContent = 'Adding your email…';
    var controller = new AbortController();
    var timeout = setTimeout(function () { controller.abort(); }, 15000);
    try {
      var result = await fetch(form.action, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          email: form.elements.email.value,
          company: form.elements.company.value,
          consent: form.elements.consent.checked
        }),
        signal: controller.signal
      });
      var body = await result.json();
      if (result.ok && body.ok === true) {
        status.dataset.state = 'success';
        status.textContent = "You're on the list. We'll email you when OpenWalkieTalkie is available to buy.";
        form.reset();
      } else {
        status.dataset.state = 'error';
        status.textContent = body.message || "We couldn't save your email. Please try again.";
      }
    } catch (error) {
      status.dataset.state = 'error';
      status.textContent = 'Connection trouble. Please try again; repeat signups are safe.';
    } finally {
      clearTimeout(timeout);
      pending = false;
      button.disabled = false;
      button.textContent = 'Join the waitlist';
      form.removeAttribute('aria-busy');
    }
  });
})();
