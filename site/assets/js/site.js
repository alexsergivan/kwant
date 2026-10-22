// kwant site: small progressive enhancements. Everything works without this file.
(function () {
  "use strict";
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  // Demo videos play while on screen, unless the visitor prefers reduced motion.
  var videos = document.querySelectorAll("video[data-autoplay]");
  if (videos.length && "IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        var v = e.target;
        if (e.isIntersecting && !reduce.matches) {
          if (v.preload === "none") v.preload = "auto";
          var p = v.play();
          if (p && p.catch) p.catch(function () { v.controls = true; });
        } else {
          v.pause();
        }
      });
    }, { threshold: 0.2 });
    videos.forEach(function (v) {
      v.controls = reduce.matches;
      io.observe(v);
    });
  }

  // On small screens the docs nav is a sideways strip: keep the current page's tab in view.
  var current = document.querySelector(".docs-nav a[aria-current]");
  if (current) {
    var strip = current.closest(".docs-nav");
    if (strip.scrollWidth > strip.clientWidth) strip.scrollLeft = current.offsetLeft - strip.offsetLeft - 18;
  }

  // Scrollable tables can be reached and scrolled with the keyboard.
  document.querySelectorAll(".table-wrap").forEach(function (w) {
    if (w.scrollWidth > w.clientWidth) {
      w.tabIndex = 0;
      w.setAttribute("role", "region");
      w.setAttribute("aria-label", "Table, scrolls sideways");
    }
  });

  // Copy buttons on code blocks.
  document.querySelectorAll("pre > code").forEach(function (code) {
    if (!navigator.clipboard) return;
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy";
    btn.textContent = "Copy";
    btn.addEventListener("click", function () {
      var text = code.innerText.replace(/\n$/, "");
      navigator.clipboard.writeText(text).then(function () {
        btn.textContent = "Copied";
        setTimeout(function () { btn.textContent = "Copy"; }, 1600);
      });
    });
    code.parentNode.appendChild(btn);
  });

  // Screenshot lightbox. Links still open the full-size image without JS.
  var dlg = document.createElement("dialog");
  if (typeof dlg.showModal !== "function") return;
  dlg.className = "lightbox";
  dlg.innerHTML = '<img alt=""><div class="lb-bar"><span></span><button type="button">Close</button></div>';
  document.body.appendChild(dlg);
  var img = dlg.querySelector("img");
  var cap = dlg.querySelector("span");
  dlg.querySelector("button").addEventListener("click", function () { dlg.close(); });
  dlg.addEventListener("click", function (e) { if (e.target === dlg) dlg.close(); });
  dlg.addEventListener("close", function () { img.removeAttribute("src"); });
  document.querySelectorAll("a.zoom").forEach(function (a) {
    a.addEventListener("click", function (e) {
      if (e.metaKey || e.ctrlKey || e.shiftKey) return;
      e.preventDefault();
      var inner = a.querySelector("img");
      img.src = a.getAttribute("href");
      img.alt = inner ? inner.alt : "";
      cap.textContent = a.getAttribute("data-caption") || img.alt;
      dlg.showModal();
    });
  });
})();
