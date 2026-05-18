// Inject PWA head tags early, before Dioxus hydration.
// Browsers check for these in the static HTML to determine installability.
(function() {
  var head = document.head;

  // Manifest link
  if (!head.querySelector('link[rel="manifest"]')) {
    var m = document.createElement('link');
    m.rel = 'manifest';
    m.href = '/manifest.json';
    head.appendChild(m);
  }

  // Theme color
  if (!head.querySelector('meta[name="theme-color"]')) {
    var tc = document.createElement('meta');
    tc.name = 'theme-color';
    tc.content = '#3b82f6';
    head.appendChild(tc);
  }

  // Apple touch icon
  if (!head.querySelector('link[rel="apple-touch-icon"]')) {
    var ai = document.createElement('link');
    ai.rel = 'apple-touch-icon';
    ai.href = '/icons/icon-192.png';
    head.appendChild(ai);
  }

  // Apple mobile web app capable
  if (!head.querySelector('meta[name="apple-mobile-web-app-capable"]')) {
    var awac = document.createElement('meta');
    awac.name = 'apple-mobile-web-app-capable';
    awac.content = 'yes';
    head.appendChild(awac);
  }

  // Apple status bar style
  if (!head.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]')) {
    var asb = document.createElement('meta');
    asb.name = 'apple-mobile-web-app-status-bar-style';
    asb.content = 'black-translucent';
    head.appendChild(asb);
  }
})();

// Register service worker
if ('serviceWorker' in navigator) {
  window.addEventListener('load', function() {
    navigator.serviceWorker.register('/sw.js', { scope: '/' });
  });
  // Reload when a new service worker takes over (deploy detected)
  navigator.serviceWorker.addEventListener('controllerchange', function() {
    window.location.reload();
  });
}
