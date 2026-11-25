/* Logo pre-paint: <script src="logo-init.js"></script> in <head> (plain, not module/deferred) when the page has no theme-init.js. */
(function () {
  try { var logo = localStorage.getItem("logo-form"); if (logo === "short" || logo === "middle" || logo === "long") document.documentElement.setAttribute("data-logo", logo); } catch (e) {}
})();
