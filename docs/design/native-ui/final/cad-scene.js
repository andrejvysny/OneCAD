/* OneCAD viewport diagram renderer. <cad-scene state="idle|face|edge|extrude|fillet|rollback|sketch|thumb" grid="1"></cad-scene>
   Same dimetric projection as the icon set: screen = ((x-y)*0.9, (x+y)*0.42 - z). Units = mm. */
(function () {
  var KX = 0.9, KY = 0.42, D = Math.PI / 180;
  var C = {
    top: '#D9DCE1', left: '#B8BDC4', right: '#A0A6AF', edge: '#3A3F47', hole: '#868C95', holeIn: '#6F757E',
    grid: '#C9CCD2', gridMaj: '#A9ADB6', bg: '#EEF0F3', ax: '#C45C5C', ay: '#5CA05C', az: '#5078BE',
    sel: '#2E6FE0', selFill: '#BCD3F7', hov: '#00ACC1', newF: '#F7A548', newS: '#D97A16',
    sk: '#2D7FF9', skDone: '#1F9D55', skSel: '#D97706', ref: '#6B7A8F', sketchBg: '#F4F7FC', sketchGrid: '#D5DEEC', sketchGridMaj: '#B7C6DE'
  };
  function P(p) { return [(p[0] - p[1]) * KX, (p[0] + p[1]) * KY - p[2]]; }
  function pts(a) { return a.map(function (p) { var q = P(p); return q[0].toFixed(2) + ',' + q[1].toFixed(2); }).join(' '); }
  function poly(a, f, s, w, extra) { return '<polygon points="' + pts(a) + '" fill="' + f + '" stroke="' + (s || 'none') + '" stroke-width="' + (w || 1) + '" vector-effect="non-scaling-stroke" stroke-linejoin="round" ' + (extra || '') + '/>'; }
  function line(a, s, w, extra) { return '<polyline points="' + pts(a) + '" fill="none" stroke="' + s + '" stroke-width="' + (w || 1) + '" vector-effect="non-scaling-stroke" stroke-linecap="round" stroke-linejoin="round" ' + (extra || '') + '/>'; }
  function circ(c, r, ax, n) { var o = []; n = n || 40; for (var i = 0; i < n; i++) { var a = i / n * 360 * D, u = r * Math.cos(a), v = r * Math.sin(a); o.push(ax === 'z' ? [c[0] + u, c[1] + v, c[2]] : [c[0] + u, c[1], c[2] + v]); } return o; }

  /* bracket: base 80x50x8, upright 80x8x42 at back (y 0..8), 2 holes in base, 1 bore in upright */
  function bracket(st) {
    var s = '', fillet = st !== 'rollback' && st !== 'thumbA', R = 6;
    var faceSel = st === 'face', edgeSel = st === 'edge' || st === 'fillet';
    var baseTop = faceSel ? C.selFill : C.top, bs = faceSel ? C.sel : C.edge;
    s += poly([[80, 0, 0], [80, 50, 0], [80, 50, 8], [80, 0, 8]], C.right, C.edge);
    s += poly([[0, 50, 0], [80, 50, 0], [80, 50, 8], [0, 50, 8]], C.left, C.edge);
    s += poly([[0, 8, 8], [80, 8, 8], [80, 50, 8], [0, 50, 8]], baseTop, bs, faceSel ? 2 : 1);
    [[16, 34], [64, 34]].forEach(function (h) {
      s += poly(circ([h[0], h[1], 8], 5, 'z'), C.hole, faceSel ? C.sel : C.edge, 1);
      s += poly(circ([h[0] + 0.9, h[1] + 0.9, 8], 4.2, 'z'), C.holeIn, 'none');
    });
    s += poly([[80, 0, 8], [80, 8, 8], [80, 8, 50], [80, 0, 50]], C.right, C.edge);
    s += poly([[0, 8, 8], [80, 8, 8], [80, 8, 50], [0, 8, 50]], C.left, C.edge);
    s += poly([[0, 0, 50], [80, 0, 50], [80, 8, 50], [0, 8, 50]], C.top, C.edge);
    s += poly(circ([40, 8, 30], 9, 'y'), C.hole, C.edge);
    s += poly(circ([40.6, 8, 29.4], 7.6, 'y'), C.holeIn, 'none');
    if (fillet || st === 'fillet') {
      var a = [], i; for (i = 0; i <= 8; i++) { var t = i / 8 * 90 * D; a.push([8 + R - R * Math.cos(t), 8 + R - R * Math.sin(t)]); }
      var strip = a.map(function (q) { return [0, q[0], q[1]]; }).concat(a.slice().reverse().map(function (q) { return [80, q[0], q[1]]; }));
      if (st === 'fillet') s += poly(strip, C.newF, C.newS, 1.5, 'fill-opacity="0.55"');
      else s += poly(strip, '#C9CDD3', C.edge, 1);
      if (st !== 'fillet') { s += line(a.map(function (q) { return [80, q[0], q[1]]; }), C.edge, 1); }
    }
    if (edgeSel) s += line([[0, 8, 8], [80, 8, 8]], st === 'fillet' ? C.newS : C.sel, 3);
    if (st === 'extrude') {
      var c0 = circ([40, 30, 8], 7, 'z', 48), c1 = circ([40, 30, 22], 7, 'z', 48);
      s += poly(c0, C.newF, C.newS, 1.5, 'fill-opacity="0.35"');
      var side = []; for (var k = 0; k <= 24; k++) { var ang = (-45 + k / 24 * 180) * D; side.push([40 + 7 * Math.cos(ang), 30 + 7 * Math.sin(ang)]); }
      var wall = side.map(function (q) { return [q[0], q[1], 8]; }).concat(side.slice().reverse().map(function (q) { return [q[0], q[1], 22]; }));
      s += poly(wall, C.newF, 'none', 1, 'fill-opacity="0.45"');
      s += poly(c1, '#FFD3A1', C.newS, 1.5, 'fill-opacity="0.85"');
      s += line([[40, 30, 22], [40, 30, 34]], C.newS, 2.5);
      var tip = P([40, 30, 36.5]); s += '<polygon points="' + tip[0] + ',' + tip[1] + ' ' + (tip[0] - 2.4) + ',' + (tip[1] + 4) + ' ' + (tip[0] + 2.4) + ',' + (tip[1] + 4) + '" fill="' + C.newS + '"/>';
      var kn = P([40, 30, 22]); s += '<circle cx="' + kn[0] + '" cy="' + kn[1] + '" r="1.6" fill="#fff" stroke="' + C.newS + '" stroke-width="1.5" vector-effect="non-scaling-stroke"/>';
    }
    return s;
  }
  function grid(sk) {
    var s = '', i, g = sk ? C.sketchGrid : C.grid, gm = sk ? C.sketchGridMaj : C.gridMaj;
    for (i = -200; i <= 300; i += 10) {
      var maj = i % 50 === 0;
      s += line([[i, -200, 0], [i, 300, 0]], maj ? gm : g, maj ? 1 : 0.6, 'stroke-opacity="' + (maj ? 0.9 : 0.7) + '"');
      s += line([[-200, i, 0], [300, i, 0]], maj ? gm : g, maj ? 1 : 0.6, 'stroke-opacity="' + (maj ? 0.9 : 0.7) + '"');
    }
    return s;
  }
  function axes() { return line([[0, 0, 0], [-14, 0, 0]], C.ax, 1.5, 'stroke-dasharray="3 3"') + line([[0, 0, 0], [0, -14, 0]], C.ay, 1.5, 'stroke-dasharray="3 3"') + line([[0, 0, 0], [0, 0, 14]], C.az, 1.5, 'stroke-dasharray="3 3"'); }

  /* sketch view: looking down -Z on the base top face */
  function sketch(variant) {
    var s = '', i;
    for (i = -200; i <= 300; i += 5) { var m = i % 25 === 0; s += '<line x1="' + i + '" y1="-200" x2="' + i + '" y2="300" stroke="' + (m ? C.sketchGridMaj : C.sketchGrid) + '" stroke-width="' + (m ? 1 : 0.6) + '" vector-effect="non-scaling-stroke"/>'; s += '<line x1="-200" y1="' + i + '" x2="300" y2="' + i + '" stroke="' + (m ? C.sketchGridMaj : C.sketchGrid) + '" stroke-width="' + (m ? 1 : 0.6) + '" vector-effect="non-scaling-stroke"/>'; }
    var R = function (x, y, w, h, c, wd, ex) { return '<rect x="' + x + '" y="' + y + '" width="' + w + '" height="' + h + '" fill="none" stroke="' + c + '" stroke-width="' + (wd || 1.5) + '" vector-effect="non-scaling-stroke" ' + (ex || '') + '/>'; };
    var L = function (x1, y1, x2, y2, c, wd, ex) { return '<line x1="' + x1 + '" y1="' + y1 + '" x2="' + x2 + '" y2="' + y2 + '" stroke="' + c + '" stroke-width="' + (wd || 1.5) + '" vector-effect="non-scaling-stroke" stroke-linecap="round" ' + (ex || '') + '/>'; };
    var Ci = function (x, y, r, c, wd, f) { return '<circle cx="' + x + '" cy="' + y + '" r="' + r + '" fill="' + (f || 'none') + '" stroke="' + c + '" stroke-width="' + (wd || 1.5) + '" vector-effect="non-scaling-stroke"/>'; };
    var T = function (x, y, t, c) { return '<text x="' + x + '" y="' + y + '" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="1.8" fill="' + (c || '#1B1D21') + '" text-anchor="middle" dominant-baseline="middle">' + t + '</text>'; };
    var tag = function (x, y, w, t) { return '<rect x="' + (x - w / 2) + '" y="' + (y - 1.5) + '" width="' + w + '" height="3" rx="0.6" fill="#fff" stroke="#CBDCF7" stroke-width="1" vector-effect="non-scaling-stroke"/>' + T(x, y + 0.2, t); };
    /* reference: base face outline (locked) */
    s += '<rect x="0" y="8" width="80" height="42" fill="#E6EBF3" fill-opacity=".6" stroke="' + C.ref + '" stroke-width="1.2" vector-effect="non-scaling-stroke"/>';
    s += Ci(16, 34, 5, C.ref, 1.2) + Ci(64, 34, 5, C.ref, 1.2);
    /* construction centre line */
    s += L(40, 4, 40, 54, C.sk, 1, 'stroke-dasharray="4 3" stroke-opacity=".7"');
    /* slot sketch: rectangle 30 x 12 centred at (40, 30) */
    var sel = variant === 'b';
    s += L(25, 24, 55, 24, sel ? C.skSel : C.sk, sel ? 2.4 : 1.6);
    s += L(55, 24, 55, 36, C.sk, 1.6) + L(55, 36, 25, 36, C.sk, 1.6) + L(25, 36, 25, 24, C.sk, 1.6);
    s += Ci(40, 30, 3, C.skDone, 1.6);
    [[25, 24], [55, 24], [55, 36], [25, 36]].forEach(function (p) { s += Ci(p[0], p[1], 0.9, C.sk, 1.2, '#fff'); });
    s += Ci(40, 30, 0.7, C.skDone, 1, C.skDone);
    /* dimensions */
    s += L(25, 19, 55, 19, '#4A5158', 0.8) + L(25, 23, 25, 18, '#4A5158', 0.6) + L(55, 23, 55, 18, '#4A5158', 0.6) + tag(40, 19, 5, '30');
    s += L(60, 24, 60, 36, '#4A5158', 0.8) + L(56, 24, 61, 24, '#4A5158', 0.6) + L(56, 36, 61, 36, '#4A5158', 0.6) + tag(60, 30, 5, '12');
    s += L(42.2, 27.8, 46, 24, '#4A5158', 0.6) + tag(47.6, 22.4, 4.6, '⌀6');
    /* cursor crosshair + live coordinate */
    if (variant === 'a') { s += L(68, 44, 72, 44, C.sk, 1) + L(70, 42, 70, 46, C.sk, 1); }
    return s;
  }

  function fitBox(cx, cy, w, h, W, H) { var s = Math.min(W / w, H / h), vw = W / s, vh = H / s; return [cx - vw / 2, cy - vh / 2, vw, vh]; }
  function render(el) {
    var st = el.getAttribute('state') || 'idle', g = el.getAttribute('grid') !== '0', z = parseFloat(el.getAttribute('zoom')) || 1;
    var ox = parseFloat(el.getAttribute('ox')) || 0, oy = parseFloat(el.getAttribute('oy')) || 0;
        var root = el.shadowRoot || el.attachShadow({ mode: 'open' }); if (!root.querySelector('style')) root.innerHTML = '<style>:host{display:block;position:relative;width:100%;height:100%;overflow:hidden}#bg{position:absolute;inset:0}</style><div id="bg"></div>';
    var W = el.clientWidth, H = el.clientHeight, vb, body, bg, sk = st === 'sketch' || st === 'sketchB'; if (!W || !H) return;
    if (sk) { vb = fitBox(40 + ox, 30 + oy, 110 / z, 72 / z, W, H); body = sketch(st === 'sketchB' ? 'b' : 'a'); bg = C.sketchBg; }
    else { vb = fitBox(13 + ox, 2 + oy, 150 / z, 132 / z, W, H); body = (g ? grid() + axes() : '') + bracket(st); bg = g ? C.bg : 'transparent'; }
    var key = st + W + 'x' + H + z + ox + oy + g; if (el._k === key) return; el._k = key;
    var box = root.getElementById('bg'); box.style.background = bg; box.innerHTML = '<svg width="' + W + '" height="' + H + '" viewBox="' + vb.join(' ') + '" style="position:absolute;left:0;top:0;display:block">' + body + '</svg>';
  }
  if (!customElements.get('cad-scene')) customElements.define('cad-scene', class extends HTMLElement {
    static get observedAttributes() { return ['state', 'grid', 'zoom', 'ox', 'oy']; }
    connectedCallback() { var s = this; render(s); if (!s._ro && window.ResizeObserver) { s._ro = new ResizeObserver(function () { cancelAnimationFrame(s._raf); s._raf = requestAnimationFrame(function () { render(s); }); }); s._ro.observe(s); } } attributeChangedCallback() { if (this.isConnected) render(this); }
  });
})();
