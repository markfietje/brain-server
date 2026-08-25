// mdBook × mermaid: turn fenced ```mermaid code blocks into rendered
// diagrams. Self-contained — mermaid.min.js is vendored in theme/js/, no
// CDN, no preprocessor binary in CI. Runs after the DOM is ready; safe with
// the print (print.html) and search outputs since it only transforms
// `pre code.language-mermaid` nodes that exist on the page.
(function () {
  'use strict';

  function init() {
    var blocks = document.querySelectorAll('pre > code.language-mermaid');
    if (!blocks.length || typeof mermaid === 'undefined') return;

    var html = document.documentElement.className || '';
    var dark = /coal|navy|ayu/.test(html);
    var nodes = [];
    blocks.forEach(function (code) {
      var pre = code.parentElement;
      var div = document.createElement('div');
      div.className = 'mermaid';
      // textContent → the browser never interprets diagram source as HTML.
      div.textContent = code.textContent;
      pre.replaceWith(div);
      nodes.push(div);
    });

    try {
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: dark ? 'dark' : 'neutral',
        flowchart: { curve: 'basis', useMaxWidth: true }
      });
      mermaid.run({ nodes: nodes });
    } catch (e) {
      // Never break the page over a diagram: restore the source as a block.
      nodes.forEach(function (div) {
        var pre = document.createElement('pre');
        pre.textContent = div.textContent;
        div.replaceWith(pre);
      });
    }
  }

  if (document.readyState !== 'loading') {
    init();
  } else {
    document.addEventListener('DOMContentLoaded', init);
  }
})();
