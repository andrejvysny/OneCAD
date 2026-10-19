/* OneCAD icon set — Colour isometric. Usage: <cad-icon name="extrude" size="20" theme="light"></cad-icon> */
(function(){
/* =========================================================================
 * OneCAD icon set — "Colour isometric" language. Single renderer.
 *
 * GRID     24 x 24, live area 2..22. 3D parts are projected, then fitted:
 *          fixed scale U (so a 10-unit cube is ~17 px everywhere), shrunk
 *          only when a glyph would overflow its fit box, centred in it.
 * PROJECT  2:1-ish dimetric. screen = ((x-y)*0.9, (x+y)*0.42 - z).
 *          x runs right-down, y left-down, z up. Visible faces: +z top,
 *          +y left, +x right. Light from top-left: top > left > right.
 * PAINT    One ordered list of slots (painter's algorithm, author order).
 *          Faces: 3-tone fill + 1 px outline. Lines: semantic colour.
 * COLOUR   b grey body · n orange CREATED · m blue MODIFIED / sketch ·
 *          r red CONSTRAINT / danger · gr green SELECT / ok · y amber ·
 *          k steel · w paper. Axis triad: X red, Y green, Z blue.
 * ========================================================================= */
var KX = 0.9, KY = 0.42, U = 0.92, NSLOT = 56;
var D = Math.PI / 180;
function A2(cx, cy, rx, ry, a0, a1, n) { var o = []; n = n || Math.max(6, Math.round(Math.abs(a1 - a0) / 8)); for (var i = 0; i <= n; i++) { var a = (a0 + (a1 - a0) * i / n) * D; o.push([cx + rx * Math.cos(a), cy + ry * Math.sin(a)]); } return o; }
function A3(c, r, ax, a0, a1, n) {
  var o = []; n = n || Math.max(6, Math.round(Math.abs(a1 - a0) / 8));
  for (var i = 0; i <= n; i++) {
    var a = (a0 + (a1 - a0) * i / n) * D, u = r * Math.cos(a), v = r * Math.sin(a);
    o.push(ax === 'z' ? [c[0] + u, c[1] + v, c[2]] : ax === 'y' ? [c[0] + u, c[1], c[2] + v] : [c[0], c[1] + u, c[2] + v]);
  }
  return o;
}
function BEZ(p0, p1, p2, p3, n) { var o = []; n = n || 24; for (var i = 0; i <= n; i++) { var t = i / n, s = 1 - t; o.push([s * s * s * p0[0] + 3 * s * s * t * p1[0] + 3 * s * t * t * p2[0] + t * t * t * p3[0], s * s * s * p0[1] + 3 * s * s * t * p1[1] + 3 * s * t * t * p2[1] + t * t * t * p3[1]].concat(p0.length > 2 ? [s * s * s * p0[2] + 3 * s * s * t * p1[2] + 3 * s * t * t * p2[2] + t * t * t * p3[2]] : [])); } return o; }
function RR(x, y, w, h, r) { return [].concat(A2(x + w - r, y + r, r, r, -90, 0, 6), A2(x + w - r, y + h - r, r, r, 0, 90, 6), A2(x + r, y + h - r, r, r, 90, 180, 6), A2(x + r, y + r, r, r, 180, 270, 6)); }
function ALONG(x1, y1, x2, y2, w) { var dx = x2 - x1, dy = y2 - y1, L = Math.hypot(dx, dy), nx = -dy / L * w / 2, ny = dx / L * w / 2; return [[x1 + nx, y1 + ny], [x2 + nx, y2 + ny], [x2 - nx, y2 - ny], [x1 - nx, y1 - ny]]; }
function GEAR(cx, cy, n, ro, ri, ph) {
  var o = [], st = 360 / n; ph = ph || 0;
  for (var i = 0; i < n; i++) {
    var a = ph + i * st;
    [[a - st * 0.5, ri], [a - st * 0.18, ri], [a - st * 0.1, ro], [a + st * 0.1, ro], [a + st * 0.18, ri]].forEach(function (p) { o.push([cx + p[1] * Math.cos(p[0] * D), cy + p[1] * Math.sin(p[0] * D)]); });
  }
  return o;
}
function CIRC(cx, cy, r, n) { return A2(cx, cy, r, r, 0, 360, n || 36).slice(0, -1); }

/* ---------------------------------------------------------------- builder */
function Builder() {
  var b = { it: [], fit: [2, 2, 22, 22] };
  b.F = function (pts, fam, sh, o) { b.it.push({ t: 'f', d3: true, pts: pts, holes: (o && o.holes) || [], fam: fam, sh: sh, o: o || {} }); };
  b.f = function (pts, fam, sh, o) { b.it.push({ t: 'f', d3: false, pts: pts, holes: (o && o.holes) || [], fam: fam, sh: sh, o: o || {} }); };
  b.L = function (pts, k, o) { b.it.push({ t: 'l', d3: true, pts: pts, k: k, o: o || {} }); };
  b.l = function (pts, k, o) { b.it.push({ t: 'l', d3: false, pts: pts, k: k, o: o || {} }); };
  b.M = function (p, k, o) { b.it.push({ t: 'm', d3: true, pts: [p], k: k, o: o || {} }); };
  b.m = function (p, k, o) { b.it.push({ t: 'm', d3: false, pts: [p], k: k, o: o || {} }); };
  b.T = function (pts, fam, sh, w, o) { b.it.push({ t: 'tube', d3: true, pts: pts, fam: fam, sh: sh, w: w, o: o || {} }); };
  b.tu = function (pts, fam, sh, w, o) { b.it.push({ t: 'tube', d3: false, pts: pts, fam: fam, sh: sh, w: w, o: o || {} }); };
  b.box = function (x0, y0, z0, dx, dy, dz, fam, o) {
    o = o || {}; var x1 = x0 + dx, y1 = y0 + dy, z1 = z0 + dz, fm = o.fams || {};
    var fo = { ns: o.ns, op: o.op, dash: o.dash };
    b.F([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], fm.top || fam, 'top', fo);
    b.F([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], fm.left || fam, 'left', fo);
    b.F([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], fm.right || fam, 'right', fo);
  };
  b.edges = function (x0, y0, z0, dx, dy, dz, k, all, o) {
    var x1 = x0 + dx, y1 = y0 + dy, z1 = z0 + dz;
    var vis = [[[x0, y0, z1], [x1, y0, z1]], [[x1, y0, z1], [x1, y1, z1]], [[x1, y1, z1], [x0, y1, z1]], [[x0, y1, z1], [x0, y0, z1]],
      [[x0, y1, z0], [x1, y1, z0]], [[x1, y1, z0], [x1, y0, z0]], [[x0, y1, z0], [x0, y1, z1]], [[x1, y1, z0], [x1, y1, z1]], [[x1, y0, z0], [x1, y0, z1]]];
    var hid = [[[x0, y0, z0], [x1, y0, z0]], [[x0, y0, z0], [x0, y1, z0]], [[x0, y0, z0], [x0, y0, z1]]];
    vis.forEach(function (e) { b.L(e, k, o); });
    if (all) hid.forEach(function (e) { b.L(e, all === true ? k : all, o); });
  };
  b.cyl = function (cx, cy, z0, h, r, fam, o) {
    o = o || {};
    b.F(A3([cx, cy, z0], r, 'z', -45, 45).concat(A3([cx, cy, z0 + h], r, 'z', 45, -45)), o.side || fam, 'right', o);
    b.F(A3([cx, cy, z0], r, 'z', 45, 135).concat(A3([cx, cy, z0 + h], r, 'z', 135, 45)), o.side || fam, 'left', o);
    if (!o.noTop) b.F(A3([cx, cy, z0 + h], r, 'z', 0, 360, 40).slice(0, -1), o.top || fam, 'top', o);
  };
  /* Extruded 2D outline (xy) from z0 to z0+h: visible side quads sorted back-to-front, hole walls, top. */
  b.prism = function (pts, z0, h, fam, o) {
    o = o || {}; var z1 = z0 + h, n = pts.length, area = 0;
    for (var i = 0; i < n; i++) { var p = pts[i], q = pts[(i + 1) % n]; area += p[0] * q[1] - q[0] * p[1]; }
    var sg = area > 0 ? 1 : -1, quads = [];
    for (var j = 0; j < n; j++) {
      var a = pts[j], c = pts[(j + 1) % n], dx = c[0] - a[0], dy = c[1] - a[1];
      var nx = dy * sg, ny = -dx * sg;
      if (nx + ny > 1e-6) quads.push({ k: (a[0] + c[0] + a[1] + c[1]) / 2, pts: [[a[0], a[1], z0], [c[0], c[1], z0], [c[0], c[1], z1], [a[0], a[1], z1]], sh: nx > ny ? 'right' : 'left' });
    }
    quads.sort(function (u, v) { return u.k - v.k; });
    quads.forEach(function (q) { b.F(q.pts, o.side || fam, q.sh); });
    (o.holes || []).forEach(function (hp) {
      var m = hp.length, ar = 0;
      for (var i = 0; i < m; i++) { var p = hp[i], q = hp[(i + 1) % m]; ar += p[0] * q[1] - q[0] * p[1]; }
      var s2 = ar > 0 ? 1 : -1, wall = [];
      for (var k = 0; k < m; k++) {
        var a = hp[k], c = hp[(k + 1) % m], dx = c[0] - a[0], dy = c[1] - a[1], nx = dy * s2, ny = -dx * s2;
        if (-(nx + ny) > 1e-6) wall.push({ k: -(a[0] + c[0] + a[1] + c[1]), pts: [[a[0], a[1], z1], [c[0], c[1], z1], [c[0], c[1], z1 - (o.holeDepth || h)], [a[0], a[1], z1 - (o.holeDepth || h)]] });
      }
      wall.sort(function (u, v) { return u.k - v.k; });
      wall.forEach(function (w) { b.F(w.pts, o.holeFam || o.side || fam, 'right', { ns: true }); });
    });
    b.F(pts.map(function (p) { return [p[0], p[1], z1]; }), o.top || fam, 'top', { holes: (o.holes || []).map(function (hp) { return hp.map(function (p) { return [p[0], p[1], z1]; }); }) });
  };
  /* 2D helpers */
  b.pencil = function (x1, y1, x2, y2, w) {
    var dx = x2 - x1, dy = y2 - y1, L = Math.hypot(dx, dy), ux = dx / L, uy = dy / L, nx = -uy * w / 2, ny = ux * w / 2;
    var P = function (t, s) { return [x1 + ux * t + nx * s, y1 + uy * t + ny * s]; };
    var e = 2.2, c = L - 4.4, g = L - 1.6;
    b.f([P(0, 1), P(e, 1), P(e, -1), P(0, -1)], 'r', 'top');
    b.f([P(e, 1), P(c, 1), P(c, -1), P(e, -1)], 'n', 'left');
    b.f([P(c, 1), P(g, 0.36), P(g, -0.36), P(c, -1)], 'y', 'top');
    b.f([P(g, 0.36), P(L, 0), P(g, -0.36)], 'k', 'right', { ns: true });
  };
  b.badge = function (kind, cx, cy, r) {
    var fam = { ok: 'gr', warn: 'y', err: 'r', info: 'm', off: 'k', add: 'n' }[kind] || 'm';
    b.f(CIRC(cx, cy, r, 24), fam, 'left');
    var s = r * 0.5;
    if (kind === 'ok') b.l([[cx - s, cy + 0.05 * r], [cx - s * 0.25, cy + s * 0.75], [cx + s, cy - s * 0.7]], 'white', { w: r * 0.36 });
    if (kind === 'err') { b.l([[cx - s * 0.8, cy - s * 0.8], [cx + s * 0.8, cy + s * 0.8]], 'white', { w: r * 0.36 }); b.l([[cx + s * 0.8, cy - s * 0.8], [cx - s * 0.8, cy + s * 0.8]], 'white', { w: r * 0.36 }); }
    if (kind === 'warn') { b.l([[cx, cy - s], [cx, cy + s * 0.25]], 'ink', { w: r * 0.34, fixed: '#2b323c' }); b.m([cx, cy + s * 0.85], 'dotK', { r: r * 0.2 }); }
    if (kind === 'add') { b.l([[cx - s, cy], [cx + s, cy]], 'white', { w: r * 0.36 }); b.l([[cx, cy - s], [cx, cy + s]], 'white', { w: r * 0.36 }); }
    if (kind === 'off') b.l([[cx - s * 0.85, cy + s * 0.85], [cx + s * 0.85, cy - s * 0.85]], 'white', { w: r * 0.34 });
  };
  return b;
}

/* ---------------------------------------------------------------- glyphs */
var G = {};
/* ===== MODEL TOOLS ===== */
G.select = function (b) { b.f([[7, 2.5], [7, 19], [11, 15.2], [13.6, 21], [16.5, 19.7], [14, 14], [19.3, 13.8]], 'w', 'top'); };
/* Planes are drawn tipped 23° toward the viewer (TP) so they keep height at 16 px. */
function TP(x, y, z) { return [x, y, -0.42 * y + (z || 0)]; }
function plane(b, s, fam, o) { b.F([TP(0, 0), TP(s, 0), TP(s, s), TP(0, s)], fam, 'top', o); }
/* Planes (v2): a solid, light-filled parallelogram with the dark outline, drawn flat on the
   grid. The see-through tipped plane of v1 was too faint at toolbar sizes. */
function PARA(x, y, w, h, k) { return [[x, y + h], [x + w, y + h], [x + w + k, y], [x + k, y]]; }
/* Two-part pencil: orange body + light cone with a dark point. */
function pencil2(b, x1, y1, x2, y2, w) {
  var dx = x2 - x1, dy = y2 - y1, L = Math.hypot(dx, dy), ux = dx / L, uy = dy / L, nx = -uy * w / 2, ny = ux * w / 2;
  var P = function (t, s) { return [x1 + ux * t + nx * s, y1 + uy * t + ny * s]; };
  var c = L - 4.2, g = L - 1.5;
  b.f([P(0, 1), P(c, 1), P(c, -1), P(0, -1)], 'n', 'left');
  b.f([P(c, 1), P(L, 0), P(c, -1)], 'y', 'top');
  b.f([P(g, 0.36), P(L, 0), P(g, -0.36)], 'k', 'right', { ns: true });
}
/* New sketch (v3, after the reference): a dotted construction corner, one blue sketch curve,
   and an orange "+" for create. Pure line glyph, three parts. */
G.sketch = function (b) {
  b.l([[3.5, 3.5], [20.5, 3.5]], 'dots'); b.l([[3.5, 6.5], [3.5, 20.5]], 'dots');
  b.l(A2(20.5, 20.5, 14.5, 14.5, 180, 270, 24), 'sk', { w: 2.2 });
  b.l([[15.5, 11.5], [15.5, 19.5]], 'new', { w: 2.2 }); b.l([[11.5, 15.5], [19.5, 15.5]], 'new', { w: 2.2 });
};
/* Datum plane: solid orange (created) sheet with its normal rising from the centre. */
G.datum = function (b) {
  b.f(PARA(2, 11, 14, 9, 6), 'n', 'top');
  b.l([[12, 15.5], [12, 2.5]], 'new', { head: 1, w: 2.2 });
  b.m([12, 15.5], 'pt', { detail: true });
};
G.extrude = function (b) {
  b.box(0, 0, 0, 12, 12, 3, 'b');
  b.box(2.5, 2.5, 3, 7, 7, 5.5, 'n');
  b.L([[6, 6, 8.5], [6, 6, 15]], 'new', { head: 1 });
};
/* Turn arrow for Revolve: a blue arc floating above the body, running round the back of the axis
   and ending front-right, with a background-coloured casing so it separates from the orange body. */
function turn(b, z, r, a0, a1, w) {
  var arc = A3([0, 0, z], r, 'z', a0, a1, 30);
  b.L(arc, 'gap', { head: 1, w: w + 1.8 });
  b.L(arc, 'mod', { head: 1, w: w });
}
G.revolve = function (b) {
  var r = 6.2, h = 8.5;
  b.F(A3([0, 0, 0], r, 'z', -45, 0).concat(A3([0, 0, h], r, 'z', 0, -45)), 'n', 'right');
  b.F(A3([0, 0, 0], r, 'z', 90, 135).concat(A3([0, 0, h], r, 'z', 135, 90)), 'n', 'left');
  b.F([[0, 0, 0], [r, 0, 0], [r, 0, h], [0, 0, h]], 'n', 'left');
  b.F([[0, 0, 0], [0, r, 0], [0, r, h], [0, 0, h]], 'm', 'right');
  b.F([[0, 0, h]].concat(A3([0, 0, h], r, 'z', 90, 360, 34)), 'n', 'top');
  b.L([[0, 0, -3], [0, 0, h + 6]], 'axis');
  turn(b, h + 3.1, 5.6, 100, 350, 2.1);
};
/* Compact revolve: whole orange cylinder on a solid axis with the same blue turn arrow; no cut-away. */
G.revolve16 = function (b) {
  b.cyl(0, 0, 0, 7, 5.2, 'n');
  b.L([[0, 0, -3.5], [0, 0, 14]], 'mod', { w: 1.6 });
  turn(b, 10.8, 5.4, 100, 350, 2.2);
};
G.fillet = function (b) {
  var s = 10, r = 4.6, c = s - r;
  b.F([[0, 0, s], [c, 0, s], [c, s, s], [0, s, s]], 'b', 'top');
  b.F([[s, 0, 0], [s, s, 0], [s, s, c], [s, 0, c]], 'b', 'right');
  b.F(A3([c, 0, c], r, 'y', 0, 90, 10).concat(A3([c, s, c], r, 'y', 90, 0, 10)), 'm', 'left');
  b.F([[0, s, 0], [s, s, 0]].concat(A3([c, s, c], r, 'y', 0, 90, 10)).concat([[0, s, s]]), 'b', 'left');
};
G.chamfer = function (b) {
  var s = 10, k = 4.6, c = s - k;
  b.F([[0, 0, s], [c, 0, s], [c, s, s], [0, s, s]], 'b', 'top');
  b.F([[s, 0, 0], [s, s, 0], [s, s, c], [s, 0, c]], 'b', 'right');
  b.F([[s, 0, c], [c, 0, s], [c, s, s], [s, s, c]], 'm', 'left');
  b.F([[0, s, 0], [s, s, 0], [s, s, c], [c, s, s], [0, s, s]], 'b', 'left');
};
G.union = function (b) {
  b.box(0, 0, 0, 7, 7, 11, 'b');
  b.box(7, 1.5, 0, 7, 7, 6, 'm');
};
G.subtract = function (b) {
  var a = 6, z = 4.5, s = 12, h = 9;
  b.F([[0, 0, h], [s, 0, h], [s, a, h], [a, a, h], [a, s, h], [0, s, h]], 'b', 'top');
  b.F([[a, a, z], [s, a, z], [s, s, z], [a, s, z]], 'm', 'top');
  b.F([[a, a, z], [a, s, z], [a, s, h], [a, a, h]], 'm', 'right');
  b.F([[a, a, z], [s, a, z], [s, a, h], [a, a, h]], 'm', 'left');
  b.F([[0, s, 0], [s, s, 0], [s, s, z], [a, s, z], [a, s, h], [0, s, h]], 'b', 'left');
  b.F([[s, 0, 0], [s, s, 0], [s, s, z], [s, a, z], [s, a, h], [s, 0, h]], 'b', 'right');
  b.edges(a, a, h, 6, 6, 3, 'ghost', false, { detail: true });
};
/* Intersect: two translucent inputs (slab + post); only their overlap is solid orange. */
G.intersect = function (b) {
  b.box(0, 0, 0, 13, 13, 4.5, 'b', { op: 0.3, so: 0.45 });
  b.box(4, 4, 4.5, 5, 5, 7.5, 'b', { op: 0.3, so: 0.45 });
  b.box(4, 4, 0, 5, 5, 4.5, 'n');
};
G.shell = function (b) {
  var s = 11, t = 1.8, i0 = t, i1 = s - t;
  b.F([[i0, i0, t], [i1, i0, t], [i1, i1, t], [i0, i1, t]], 'm', 'top');
  b.F([[i0, i0, t], [i0, i1, t], [i0, i1, s], [i0, i0, s]], 'm', 'right');
  b.F([[i0, i0, t], [i1, i0, t], [i1, i0, s], [i0, i0, s]], 'm', 'left');
  b.F([[0, 0, s], [s, 0, s], [s, s, s], [0, s, s]], 'b', 'top', { holes: [[[i0, i0, s], [i1, i0, s], [i1, i1, s], [i0, i1, s]]] });
  b.F([[0, s, 0], [s, s, 0], [s, s, s], [0, s, s]], 'b', 'left');
  b.F([[s, 0, 0], [s, s, 0], [s, s, s], [s, 0, s]], 'b', 'right');
};
G.offsetFace = function (b) {
  b.box(0, 0, 0, 12, 12, 6, 'b', { fams: { top: 'm' } });
  b.edges(0, 0, 6, 12, 12, 4, 'ghost', false, { detail: true });
  b.L([[6, 6, 6.2], [6, 6, 13.5]], 'mod', { head: 2 });
};
G.hole = function (b) {
  var s = 12, h = 7, r = 3.4, d = 4.2;
  b.F(A3([6, 6, h - d], r, 'z', 0, 360, 36).slice(0, -1), 'm', 'top');
  b.F(A3([6, 6, h], r, 'z', 135, 315, 20).concat(A3([6, 6, h - d], r, 'z', 315, 135, 20)), 'm', 'right');
  b.F([[0, 0, h], [s, 0, h], [s, s, h], [0, s, h]], 'b', 'top', { holes: [A3([6, 6, h], r, 'z', 0, 360, 36).slice(0, -1)] });
  b.F([[0, s, 0], [s, s, 0], [s, s, h], [0, s, h]], 'b', 'left');
  b.F([[s, 0, 0], [s, s, 0], [s, s, h], [s, 0, h]], 'b', 'right');
  b.L([[6, 6, 15.5], [6, 6, 8.2]], 'mod', { head: 1 });
};
G.patternLinear = function (b) {
  b.box(0, 0, 0, 4.6, 4.6, 4.6, 'b');
  b.box(6.6, 0, 0, 4.6, 4.6, 4.6, 'm');
  b.box(13.2, 0, 0, 4.6, 4.6, 4.6, 'm');
  b.L([[0, 7.6, 0], [17.8, 7.6, 0]], 'mod', { head: 1 });
};
G.patternCircular = function (b) {
  b.L(A3([0, 0, 0], 4.2, 'z', 110, 330), 'mod', { head: 1, detail: true });
  var cubes = [];
  for (var i = 0; i < 6; i++) { var a = (90 + i * 60) * D; cubes.push({ x: 7.2 * Math.cos(a), y: 7.2 * Math.sin(a), src: i === 0 }); }
  cubes.sort(function (p, q) { return (p.x + p.y) - (q.x + q.y); });
  cubes.forEach(function (c) { b.box(c.x - 1.9, c.y - 1.9, 0, 3.8, 3.8, 3.8, c.src ? 'b' : 'm'); });
  b.L([[0, 0, -1.5], [0, 0, 7]], 'axis');
};
G.patternLinear16 = function (b) {
  b.box(0, 0, 0, 6, 6, 6, 'b'); b.box(9, 0, 0, 6, 6, 6, 'm');
  b.L([[0, 9.5, 0], [16, 9.5, 0]], 'mod', { head: 1 });
};
G.patternCircular16 = function (b) {
  var cubes = [];
  for (var i = 0; i < 4; i++) { var a = (90 + i * 90) * D; cubes.push({ x: 6.4 * Math.cos(a), y: 6.4 * Math.sin(a), src: i === 0 }); }
  cubes.sort(function (p, q) { return (p.x + p.y) - (q.x + q.y); });
  cubes.forEach(function (c) { b.box(c.x - 2.4, c.y - 2.4, 0, 4.8, 4.8, 4.8, c.src ? 'b' : 'm'); });
};
/* Mirror (v5): two aligned cubes on the model X axis. The mirror plane stands halfway between
   them, parallel to the two faces that look at each other (x = const), and overhangs the cubes. */
G.mirror = function (b) {
  b.box(0, 0, 0, 6, 6, 6, 'b');
  b.F([[9, -2, -2], [9, 8, -2], [9, 8, 8.5], [9, -2, 8.5]], 'q', 'right', { op: 0.55 });
  b.box(12, 0, 0, 6, 6, 6, 'm');
};
/* Compact intersect: two sheets overlapping on a tipped plane; only the overlap is orange. */
G.intersect16 = function (b) {
  var sq = function (x, y, s) { return [TP(x, y), TP(x + s, y), TP(x + s, y + s), TP(x, y + s)]; };
  b.F(sq(0, 0, 9), 'b', 'top', { ns: true }); b.F(sq(4.5, 4.5, 9), 'b', 'top', { ns: true });
  b.F(sq(4.5, 4.5, 4.5), 'n', 'left');
  var o = function (x, y, s) { var p = sq(x, y, s); p.push(p[0]); return p; };
  b.L(o(0, 0, 9), 'outline', { w: 1.5 }); b.L(o(4.5, 4.5, 9), 'outline', { w: 1.5 });
};
G.move = function (b) {
  b.box(0, 0, 0, 8, 8, 8, 'b');
  b.L([[4, 4, 8], [4, 4, 13.5]], 'az', { head: 1 });
  b.L([[8, 4, 4], [13.5, 4, 4]], 'ax', { head: 1 });
  b.L([[4, 8, 4], [4, 13.5, 4]], 'ay', { head: 1 });
};
G.measure = function (b) {
  b.f(ALONG(3.6, 20.4, 20.4, 3.6, 7), 'y', 'top');
  var ux = 1 / Math.SQRT2, uy = -1 / Math.SQRT2, px = ux, py = -uy;
  for (var i = 1; i <= 7; i++) {
    var t = i * 2.85, sx = 3.6 + ux * t - px * 3.5 * -1, sy = 20.4 + uy * t - py * 3.5 * -1;
    var len = i % 2 ? 2.2 : 3.6;
    b.l([[3.6 + ux * t - px * 3.5, 20.4 + uy * t - py * 3.5], [3.6 + ux * t - px * (3.5 - len), 20.4 + uy * t - py * (3.5 - len)]], 'ink', { w: 1.1, detail: i % 2 === 1 });
  }
};
/* Gear generator (v2): front-facing spur gear, one flat accent orange (created), no outline.
   Ten broad trapezoid teeth and a keyed bore keep it apart from the eight-tooth Settings cog. */
function SPUR(cx, cy, n, ro, ri, ph) {
  var o = [], st = 360 / n;
  for (var i = 0; i < n; i++) {
    var a = ph + i * st;
    [[a - st * 0.5, ri], [a - st * 0.26, ri], [a - st * 0.15, ro], [a + st * 0.15, ro], [a + st * 0.26, ri]].forEach(function (p) { o.push([cx + p[1] * Math.cos(p[0] * D), cy + p[1] * Math.sin(p[0] * D)]); });
  }
  return o;
}
function BORE(cx, cy, r, kw, kd) {
  var a = Math.asin(kw / 2 / r) / D, h = A2(cx, cy, r, r, -90 + a, 270 - a, 28);
  var e = h[h.length - 1]; h.push([e[0], cy - r - kd]); h.push([h[0][0], cy - r - kd]); return h;
}
G.gear = function (b) { b.f(SPUR(12, 12, 10, 10.6, 8.2, -90), 'orange', 'top', { ns: true, holes: [BORE(12, 12, 3.4, 1.8, 1.3)] }); };

/* ===== SKETCH TOOLS (flat) ===== */
function pts(b, list, k) { list.forEach(function (p) { b.m(p, k || 'pt'); }); }
G.line = function (b) { b.l([[4.5, 19.5], [19.5, 4.5]], 'sk'); pts(b, [[4.5, 19.5], [19.5, 4.5]]); };
G.polyline = function (b) { var p = [[3, 18], [8.5, 6.5], [14.5, 16], [21, 5.5]]; b.l(p, 'sk'); pts(b, p); };
G.rect = function (b) { b.l([[4, 6.5], [20, 6.5], [20, 17.5], [4, 17.5], [4, 6.5]], 'sk'); pts(b, [[4, 6.5], [20, 17.5]]); };
G.centerRect = function (b) { b.l([[4, 6.5], [20, 6.5], [20, 17.5], [4, 17.5], [4, 6.5]], 'sk'); b.l([[12, 12], [20, 6.5]], 'skc'); pts(b, [[12, 12], [20, 6.5]]); };
G.circle = function (b) { b.l(A2(12, 12, 8.3, 8.3, 0, 360, 48), 'sk'); b.l([[12, 12], [17.87, 6.13]], 'skc'); pts(b, [[12, 12], [17.87, 6.13]]); };
G.ellipse = function (b) { b.l(A2(12, 12, 9.3, 5.6, 0, 360, 48), 'sk'); b.l([[2.7, 12], [21.3, 12]], 'skc'); b.l([[12, 6.4], [12, 17.6]], 'skc'); pts(b, [[12, 12]]); };
G.arc = function (b) { b.l(A2(6, 18, 13, 13, -90, 0, 24), 'sk'); b.l([[6, 5], [6, 18], [19, 18]], 'skc'); pts(b, [[6, 18], [6, 5], [19, 18]]); };
G.arc3 = function (b) { b.l(A2(12, 17.5, 9, 9, 180, 360, 30), 'sk'); pts(b, [[3, 17.5], [12, 8.5], [21, 17.5]]); };
G.polygon = function (b) { var h = []; for (var i = 0; i <= 6; i++) { var a = (-90 + i * 60) * D; h.push([12 + 9 * Math.cos(a), 12.4 + 9 * Math.sin(a)]); } b.l(h, 'sk'); b.l([[12, 12.4], h[1]], 'skc'); pts(b, [[12, 12.4], h[1]]); };
G.slot = function (b) {
  var p = [].concat([[7.5, 7.5], [16.5, 7.5]], A2(16.5, 12, 4.5, 4.5, -90, 90, 12), [[7.5, 16.5]], A2(7.5, 12, 4.5, 4.5, 90, 270, 12));
  b.l(p, 'sk'); b.l([[7.5, 12], [16.5, 12]], 'skc'); pts(b, [[7.5, 12], [16.5, 12]]);
};
G.point = function (b) { b.l([[4, 12], [9, 12]], 'skc'); b.l([[15, 12], [20, 12]], 'skc'); b.l([[12, 4], [12, 9]], 'skc'); b.l([[12, 15], [12, 20]], 'skc'); b.m([12, 12], 'ptL'); };
G.spline = function (b) {
  var P = [[3, 18], [7.5, 3], [16.5, 21], [21, 6]];
  b.l(P, 'skc', { detail: true }); b.l(BEZ(P[0], P[1], P[2], P[3], 30), 'sk'); pts(b, [P[0], P[3]]); pts(b, [P[1], P[2]], 'spt');
};
G.constructionLine = function (b) { b.l([[4, 19.5], [20, 4.5]], 'cons'); pts(b, [[4, 19.5], [20, 4.5]]); };
G.dimension = function (b) {
  b.l([[5, 18], [19, 18]], 'sk'); pts(b, [[5, 18], [19, 18]]);
  b.l([[5, 15.6], [5, 6]], 'thin'); b.l([[19, 15.6], [19, 6]], 'thin');
  b.l([[5, 9.5], [19, 9.5]], 'con', { head: 2 });
};
G.trim = function (b) {
  b.l([[3, 14], [21, 14]], 'sk'); b.l([[12, 14], [12, 21]], 'sk');
  b.l([[12, 3.5], [12, 13]], 'hid');
  b.l([[9.4, 5.6], [14.6, 10.8]], 'con'); b.l([[14.6, 5.6], [9.4, 10.8]], 'con');
};
G.extend = function (b) { b.l([[19.5, 3], [19.5, 21]], 'sk'); b.l([[3.5, 13], [11, 13]], 'sk'); b.l([[11, 13], [19, 13]], 'new', { head: 1 }); pts(b, [[3.5, 13]]); };
G.sketchMirror = function (b) {
  b.l([[12, 2.5], [12, 21.5]], 'axis');
  b.l([[10, 5], [3.5, 15.5], [10, 19], [10, 5]], 'sk');
  b.l([[14, 5], [20.5, 15.5], [14, 19], [14, 5]], 'new');
};
G.sketchFillet = function (b) {
  b.l([[4.5, 4.5], [13.5, 4.5]], 'hid'); b.l([[4.5, 4.5], [4.5, 13.5]], 'hid');
  b.l([[4.5, 20.5], [4.5, 13.5]], 'sk'); b.l([[13.5, 4.5], [20.5, 4.5]], 'sk');
  b.l(A2(13.5, 13.5, 9, 9, 180, 270, 16), 'new', { w: 2.2 });
  pts(b, [[4.5, 13.5], [13.5, 4.5]]);
};
G.sketchOffset = function (b) {
  b.l([[9, 9], [16, 9], [16, 12], [12, 12], [12, 16], [9, 16], [9, 9]], 'sk');
  b.l([[4.5, 4.5], [20, 4.5], [20, 16], [16, 16], [16, 20], [4.5, 20], [4.5, 4.5]], 'new');
};
/* Project edges (v3): a grey body floating over its own footprint, drawn as a blue sketch outline.
   The outline has the body's exact isometric shape, so it reads as the body's projection. */
G.project = function (b) {
  b.L([[0, 0, 0], [8, 0, 0], [8, 8, 0], [0, 8, 0], [0, 0, 0]], 'sk', { w: 2.2 });
  b.box(0, 0, 6.5, 8, 8, 8, 'b');
};

/* ===== CONSTRAINTS (flat: blue geometry, red relation) ===== */
G.coincident = function (b) { b.l([[4, 19.5], [12, 10]], 'sk'); b.l([[20, 17.5], [12, 10]], 'sk'); pts(b, [[4, 19.5], [20, 17.5]]); b.m([12, 10], 'cpt'); };
/* Horizontal / Vertical: the blue segment lies exactly on a red dashed reference axis. */
G.horizontal = function (b) {
  b.l([[2, 12], [22, 12]], 'cond', { w: 1.5 });
  b.l([[7.5, 12], [16.5, 12]], 'sk', { w: 2.4 });
  pts(b, [[7.5, 12], [16.5, 12]]);
};
G.vertical = function (b) {
  b.l([[12, 2], [12, 22]], 'cond', { w: 1.5 });
  b.l([[12, 7.5], [12, 16.5]], 'sk', { w: 2.4 });
  pts(b, [[12, 7.5], [12, 16.5]]);
};
/* Parallel: two lines held apart by equal red rungs = constant spacing. */
G.parallel = function (b) {
  b.l([[6.5, 13.5], [11.5, 18.5]], 'con', { w: 2 }); b.l([[13.5, 6.5], [18.5, 11.5]], 'con', { w: 2 });
  b.l([[3, 16], [16, 3]], 'sk', { w: 2.2 }); b.l([[8, 21], [21, 8]], 'sk', { w: 2.2 });
};
G.perpendicular = function (b) { b.l([[3, 19.5], [21, 19.5]], 'sk'); b.l([[8, 19.5], [8, 3]], 'sk'); b.l([[8, 14], [13.5, 14], [13.5, 19.5]], 'con', { w: 2 }); };
G.tangent = function (b) { b.l(A2(12, 14, 6.8, 6.8, 0, 360, 44), 'sk'); b.l([[2.5, 7.2], [21.5, 7.2]], 'sk'); b.m([12, 7.2], 'cpt'); };
G.concentric = function (b) { b.l(A2(12, 12, 9, 9, 0, 360, 48), 'sk'); b.l(A2(12, 12, 4.8, 4.8, 0, 360, 32), 'sk'); b.m([12, 12], 'cpt'); };
G.equal = function (b) {
  b.l([[3.5, 7.5], [20.5, 7.5]], 'sk'); b.l([[3.5, 16.5], [20.5, 16.5]], 'sk');
  [[10.6, 7.5], [13.4, 7.5], [10.6, 16.5], [13.4, 16.5]].forEach(function (p) { b.l([[p[0] - 0.9, p[1] + 2.4], [p[0] + 0.9, p[1] - 2.4]], 'con', { w: 1.9 }); });
  pts(b, [[3.5, 7.5], [20.5, 7.5], [3.5, 16.5], [20.5, 16.5]]);
};
G.midpoint = function (b) {
  b.l([[3, 18], [21, 6]], 'sk'); pts(b, [[3, 18], [21, 6]]);
  b.l([[4.2, 21.5], [10.8, 17.1]], 'con', { w: 1.3, detail: true }); b.l([[13.2, 15.5], [19.8, 11.1]], 'con', { w: 1.3, detail: true });
  b.m([12, 12], 'cpt');
};
G.symmetric = function (b) { b.l([[12, 2.5], [12, 21.5]], 'axis'); pts(b, [[5, 9.5], [19, 9.5]]); b.l([[5.5, 16.5], [18.5, 16.5]], 'con', { head: 2 }); };
G.pointOnCurve = function (b) { b.l(A2(12, 22, 10.5, 10.5, 200, 340, 30), 'sk'); b.l([[12, 6.2], [12, 9.6]], 'con', { head: 1, detail: true }); b.m([12, 11.5], 'cpt'); };
G.fix = function (b) {
  b.l([[3, 20.5], [11, 12.5]], 'sk'); b.m([3, 20.5], 'pt');
  b.l(A2(16.5, 11.5, 2.8, 2.8, 180, 360, 12).concat([[19.3, 13.5]]), 'ink', { w: 2.2, prepend: [[13.7, 13.5]] });
  b.f([[12.5, 12.5], [20.5, 12.5], [20.5, 20.5], [12.5, 20.5]], 'r', 'left');
  b.m([16.5, 16.3], 'dotW', { r: 1.1 });
};
G.distance = function (b) {
  pts(b, [[4.5, 16], [19.5, 16]]);
  b.l([[4.5, 13.8], [4.5, 6.5]], 'thin'); b.l([[19.5, 13.8], [19.5, 6.5]], 'thin');
  b.l([[4.5, 9.5], [19.5, 9.5]], 'con', { head: 2 });
};
G.distanceH = function (b) {
  pts(b, [[5, 19], [19, 12.5]]);
  b.l([[5, 16.8], [5, 5]], 'thin'); b.l([[19, 10.3], [19, 5]], 'thin');
  b.l([[5, 7.5], [19, 7.5]], 'con', { head: 2 });
};
G.distanceV = function (b) {
  pts(b, [[5, 5], [12, 19]]);
  b.l([[7.2, 5], [19.5, 5]], 'thin'); b.l([[14.2, 19], [19.5, 19]], 'thin');
  b.l([[17, 5], [17, 19]], 'con', { head: 2 });
};
G.angle = function (b) {
  b.l([[4, 19], [21, 19]], 'sk'); b.l([[4, 19], [15.5, 5.5]], 'sk'); b.m([4, 19], 'pt');
  b.l(A2(4, 19, 10.5, 10.5, -49.6, -2, 14), 'con', { head: 2 });
};
G.radius = function (b) { b.l(A2(6, 18, 13, 13, -90, 0, 24), 'sk'); b.m([6, 18], 'pt'); b.l([[6, 18], [15.2, 8.8]], 'con', { head: 1 }); };
G.diameter = function (b) { b.l(A2(12, 12, 8.6, 8.6, 0, 360, 48), 'sk'); b.l([[5.9, 18.1], [18.1, 5.9]], 'con', { head: 2 }); };
G.horizontalPoints = function (b) { b.l([[2.5, 12], [21.5, 12]], 'cond'); pts(b, [[6, 12], [18, 12]]); };
G.verticalPoints = function (b) { b.l([[12, 2.5], [12, 21.5]], 'cond'); pts(b, [[12, 6], [12, 18]]); };
G.underConstrained = function (b) {
  b.l([[3.5, 7], [16, 7], [16, 18.5], [3.5, 18.5], [3.5, 7]], 'sk'); pts(b, [[3.5, 7], [3.5, 18.5], [16, 18.5]]);
  b.m([16, 7], 'pt'); b.l([[17.3, 5.7], [21.3, 1.9]], 'mod', { head: 1, w: 1.5 });
};
G.fullyConstrained = function (b) {
  b.l([[3, 5.5], [17, 5.5], [17, 16], [3, 16], [3, 5.5]], 'ink'); pts(b, [[3, 5.5], [17, 5.5], [3, 16]], 'ptK');
  b.badge('ok', 17.5, 17, 5.2);
};
G.overConstrained = function (b) {
  b.l([[3, 5.5], [17, 5.5], [17, 16], [3, 16], [3, 5.5]], 'con'); pts(b, [[3, 5.5], [17, 5.5], [3, 16]], 'cpt');
  b.f([[17.5, 11], [23, 21.5], [12, 21.5]], 'y', 'left');
  b.l([[17.5, 14.4], [17.5, 18]], 'ink', { w: 1.5, fixed: '#2b323c' }); b.m([17.5, 19.9], 'dotK', { r: 0.8 });
};
G.conflict = function (b) {
  b.l([[3, 5.5], [17, 5.5], [17, 16], [3, 16], [3, 5.5]], 'con'); pts(b, [[3, 5.5], [17, 5.5], [3, 16]], 'cpt');
  b.badge('err', 17.5, 17, 5.2);
};

/* ===== VIEW & NAVIGATION ===== */
function cubeV(b, fams) { b.box(0, 0, 0, 10, 10, 10, 'b', { fams: fams }); }
/* The named view face uses ONE flat blue (vf) whatever its orientation, so Front/Top/Right match. */
G.viewFront = function (b) { cubeV(b, { left: 'vf' }); };
G.viewTop = function (b) { cubeV(b, { top: 'vf' }); };
G.viewRight = function (b) { cubeV(b, { right: 'vf' }); };
G.viewIso = function (b) { cubeV(b, { top: 'm', left: 'm', right: 'm' }); b.M([10, 10, 10], 'pt'); };
G.home = function (b) {
  b.box(0, 0, 0, 9, 9, 6.5, 'b');
  b.F([[0, 9, 6.5], [9, 9, 6.5], [4.5, 9, 11]], 'b', 'left');
  b.F([[4.5, 0, 11], [9, 0, 6.5], [9, 9, 6.5], [4.5, 9, 11]], 'm', 'right');
  b.F([[3, 9, 0], [6, 9, 0], [6, 9, 4], [3, 9, 4]], 'm', 'left');
};
G.fit = function (b) {
  b.fit = [7, 7, 17, 17];
  b.box(0, 0, 0, 10, 10, 10, 'b');
  [[[3, 8], [3, 3], [8, 3]], [[16, 3], [21, 3], [21, 8]], [[21, 16], [21, 21], [16, 21]], [[8, 21], [3, 21], [3, 16]]].forEach(function (p) { b.l(p, 'ink', { w: 2 }); });
};
/* Isolate: the chosen body solid blue; the rest of the model fades to translucent grey behind it. */
G.isolate = function (b) {
  b.box(-6.5, 0, 0, 6, 6, 6, 'b', { op: 0.35, so: 0.35 });
  b.box(0, -6.5, 0, 6, 6, 6, 'b', { op: 0.35, so: 0.35 });
  b.box(0, 0, 0, 8, 8, 8, 'm');
};
/* Orbit (v3): atom. A small solid blue nucleus with two blue elliptical orbits crossing at +/-45 deg. */
function ER(cx, cy, rx, ry, rot, a0, a1, n) {
  var c = Math.cos(rot * D), s = Math.sin(rot * D);
  return A2(0, 0, rx, ry, a0, a1, n).map(function (p) { return [cx + p[0] * c - p[1] * s, cy + p[0] * s + p[1] * c]; });
}
/* Isolate brainstorm: four candidate metaphors, shown as option cards on the View board. */
G.isolateA = function (b) {
  var R = 7.2, ap = [3.5, 3.5, 19], q = Math.PI / 4;
  b.F([ap].concat(A3([3.5, 3.5, 0], R, 'z', -45, 135, 24)), 'y', 'top', { op: 0.45, ns: true });
  b.F(A3([3.5, 3.5, 0], R, 'z', 0, 360, 36).slice(0, -1), 'y', 'left', { op: 0.6, ns: true });
  b.L([ap, [3.5 + R * Math.cos(-q), 3.5 + R * Math.sin(-q), 0]], 'amber', { w: 1.3 });
  b.L([ap, [3.5 + R * Math.cos(3 * q), 3.5 + R * Math.sin(3 * q), 0]], 'amber', { w: 1.3 });
  b.box(0, 0, 0, 7, 7, 7, 'm');
};
G.isolateB = function (b) {
  [[[-3, -3, -3], [10, -3, -3]], [[-3, -3, -3], [-3, 10, -3]], [[-3, -3, -3], [-3, -3, 10]]].forEach(function (x) { b.L(x, 'thin', { w: 1 }); });
  b.box(0, 0, 0, 7, 7, 7, 'm');
  b.edges(-3, -3, -3, 13, 13, 13, 'ink', false, { w: 1.4 });
};
G.isolateC = function (b) {
  b.box(-7, 0, 0, 6, 6, 6, 'b', { op: 0.35, so: 0.35 });
  b.box(0, -7, 0, 6, 6, 6, 'b', { op: 0.35, so: 0.35 });
  b.box(0, 0, 0, 6, 6, 6, 'none', { op: 0, so: 0.5 });
  b.box(0, 0, 9, 6, 6, 6, 'm');
  b.L([[6, 6, 2], [6, 6, 8]], 'mod', { head: 1, w: 1.8 });
};
G.isolateD = function (b) {
  b.F(A3([3.5, 3.5, 0], 8.5, 'z', 0, 360, 40).slice(0, -1), 'q', 'top', { op: 0.7 });
  b.box(0, 0, 0, 7, 7, 7, 'm');
};
G.orbit = function (b) {
  b.l(ER(12, 12, 10.2, 4.1, 45, 0, 360, 40), 'mod', { w: 1.9 });
  b.l(ER(12, 12, 10.2, 4.1, -45, 0, 360, 40), 'mod', { w: 1.9 });
  b.f(CIRC(12, 12, 1.9, 20), 'blue', 'top', { ns: true });
};
G.pan = function (b) {
  b.l([[12, 4], [12, 20]], 'ink', { w: 1.9, head: 2 }); b.l([[4, 12], [20, 12]], 'ink', { w: 1.9, head: 2 });
  b.m([12, 12], 'ptL');
};
function lens(b, sign) {
  b.l(A2(10, 10, 6.6, 6.6, 0, 360, 36), 'ink', { w: 2.1 });
  b.l([[14.9, 14.9], [20.5, 20.5]], 'ink', { w: 2.8 });
  if (sign) { b.l([[7, 10], [13, 10]], 'ink', { w: 2 }); if (sign === '+') b.l([[10, 7], [10, 13]], 'ink', { w: 2 }); }
}
G.zoomIn = function (b) { lens(b, '+'); };
G.zoomOut = function (b) { lens(b, '-'); };
G.orthoPersp = function (b) {
  /* strong taper: the back edges are ~65% of the front, so perspective survives 16 px */
  b.l([[9, 4.5], [21.5, 2]], 'hid', { detail: true }); b.l([[16.5, 4.5], [21.5, 2]], 'mod', { w: 1.4 }); b.l([[16.5, 14.5], [21.5, 2]], 'mod', { w: 1.4 });
  b.m([21.5, 2], 'spt', { r: 1.5 });
  b.f([[2.5, 9], [14, 9], [16.5, 4.5], [9, 4.5]], 'b', 'top');
  b.f([[2.5, 9], [14, 9], [14, 21], [2.5, 21]], 'b', 'left');
  b.f([[14, 9], [16.5, 4.5], [16.5, 14.5], [14, 21]], 'b', 'right');
};
G.grid = function (b) {
  var r = RR(3.5, 3.5, 17, 17, 2.5); r.push(r[0]);
  b.l(r, 'ink', { w: 2 });
  [9.2, 14.8].forEach(function (t) { b.l([[t, 4], [t, 20]], 'ink', { w: 1.3 }); b.l([[4, t], [20, t]], 'ink', { w: 1.3 }); });
};
/* Snap (v2): monochrome magnet silhouette, sharp corners, pole caps split by a gap; no colour, no point. */
G.snap = function (b) {
  var U = [[4.5, 3], [4.5, 12.5]].concat(A2(12, 12.5, 7.5, 7.5, 180, 0, 24).slice(1))
    .concat([[19.5, 3], [14.5, 3], [14.5, 12.5]]).concat(A2(12, 12.5, 2.5, 2.5, 0, 180, 12).slice(1)).concat([[9.5, 3]]);
  b.f(U, 'ink', 'top');
  b.l([[3.5, 7.4], [20.5, 7.4]], 'gap', { w: 1.3 });
};
G.shaded = function (b) { b.box(0, 0, 0, 10, 10, 10, 'b', { ns: true }); b.L([[0, 0, 10], [10, 0, 10], [10, 0, 0], [10, 10, 0], [0, 10, 0], [0, 10, 10], [0, 0, 10]], 'outline', { w: 1.05 }); };
G.shadedEdges = function (b) { b.box(0, 0, 0, 10, 10, 10, 'b'); };
G.wireframe = function (b) { b.edges(0, 0, 0, 10, 10, 10, 'ink', true, { w: 1.3 }); };
G.hiddenLine = function (b) { b.box(0, 0, 0, 10, 10, 10, 'w'); b.edges(0, 0, 0, 10, 10, 10, 'none', 'hid'); };
G.section = function (b) {
  b.F([[0, 0, 10], [12, 0, 10], [12, 6, 10], [0, 6, 10]], 'b', 'top');
  b.F([[12, 0, 0], [12, 6, 0], [12, 6, 10], [12, 0, 10]], 'b', 'right');
  b.F([[0, 6, 0], [12, 6, 0], [12, 6, 10], [0, 6, 10]], 'r', 'top');
  [2, 5, 8, 11, 14, 17].forEach(function (t) {
    var p0 = [Math.min(t, 12), 6, Math.max(0, t - 12)], p1 = [Math.max(0, t - 10), 6, Math.min(10, t)];
    b.L([p0, p1], 'hatch', { detail: true });
  });
  b.F([[-2, 6, -2], [14, 6, -2], [14, 6, 12], [-2, 6, 12]], 'p', 'left', { op: 0.3 });
};
/* Layers: three flat sheets, generously spaced; no thickness to blur at 16 px. */
G.layers = function (b) {
  var s = 12, sh = function (z) { return [[0, 0, z], [s, 0, z], [s, s, z], [0, s, z]]; };
  b.F(sh(0), 'b', 'left'); b.F(sh(4.2), 'b', 'top'); b.F(sh(8.4), 'm', 'top');
};

/* ===== TREE & ENTITIES ===== */
G.body = function (b) { b.box(0, 0, 0, 10, 10, 10, 'b'); };
/* Sketch entity reuses the New sketch glyph so tool and tree row match. */
G.sketchEntity = function (b) { G.sketch(b); };
G.datumEntity = function (b) { b.f(PARA(2, 7, 14, 10, 6), 'n', 'top'); };
function eye(b) {
  var top = A2(12, 19.5, 11, 11, 225, 315, 14), bot = A2(12, 4.5, 11, 11, 45, 135, 14), e = top.concat(bot); e.push(e[0]);
  b.l(e, 'ink', { w: 2 });
  b.f(CIRC(12, 12, 3.4, 24), 'ink', 'top');
}
G.eyeOn = function (b) { eye(b); };
G.eyeOff = function (b) { eye(b); b.l([[4, 20], [20, 4]], 'gap', { w: 4.6 }); b.l([[4, 20], [20, 4]], 'ink', { w: 2 }); };
function lock(b, open) {
  var sh = open ? A2(12, 7.5, 4, 4, 180, 330, 14) : A2(12, 9.5, 4, 4, 180, 360, 14);
  var L = open ? [[8, 11.5], [8, 7.5]].concat(sh) : [[8, 11.5], [8, 9.5]].concat(sh).concat([[16, 11.5]]);
  b.l(L, 'ink', { w: 2.2 });
  var r = RR(5, 11.5, 14, 9.5, 1.5); r.push(r[0]); b.l(r, 'ink', { w: 2 });
  b.m([12, 16.2], 'dotI', { r: 1.5 });
}
G.lock = function (b) { lock(b, false); };
G.unlock = function (b) { lock(b, true); };
G.material = function (b) {
  b.f(CIRC(12, 12, 9, 40), 'm', 'right');
  b.f(A2(10.5, 10.5, 7.4, 7.4, 0, 360, 36).slice(0, -1), 'm', 'left', { ns: true });
  b.f(A2(9.5, 9.5, 4.2, 4.2, 0, 360, 24).slice(0, -1), 'm', 'top', { ns: true });
  b.m([8.4, 8.4], 'dotW', { r: 1.5 });
  b.l(A2(12, 12, 9, 9, 0, 360, 40), 'outline', { w: 1 });
};
G.folder = function (b) { b.l([[3, 19], [3, 5.5], [9.5, 5.5], [11.5, 7.5], [21, 7.5], [21, 19], [3, 19]], 'ink', { w: 2 }); b.l([[3, 10.5], [21, 10.5]], 'ink', { w: 1.8 }); };
G.component = function (b) { b.edges(-1.6, -1.6, -1.6, 13.2, 13.2, 13.2, 'ghost', false, { detail: true }); b.box(0, 0, 0, 10, 10, 10, 'b'); b.box(5, 5, 10, 5, 5, 3.6, 'n'); };
G.origin = function (b) {
  b.L([[0, 0, 0], [11, 0, 0]], 'ax', { head: 1 }); b.L([[0, 0, 0], [0, 11, 0]], 'ay', { head: 1 }); b.L([[0, 0, 0], [0, 0, 11]], 'az', { head: 1 });
  b.M([0, 0, 0], 'ptL');
};
G.construction = function (b) {
  b.f(PARA(2, 9, 14, 9, 6), 'none', 'top');
  b.l([[12, 2.5], [12, 21.5]], 'cons', { w: 1.9 });
  b.m([12, 13.5], 'pt');
};

/* ===== ROADMAP OPERATIONS ===== */
/* Sweep (v2): a square profile (blue cap on top) swept along an L-shaped path in the XZ plane:
   down, a 90 deg bend, then along +x. A true solid elbow: flat +y side, inner bent surface, orange end cap. */
function elbow(b, H, r, L2, t, d, fam, pfam) {
  var P = [], i;
  for (i = 0; i <= 3; i++) P.push([0, H - (H - r) * i / 3]);
  P = P.concat(A2(r, r, r, r, 180, 90, 10).slice(1).map(function (q) { return [q[0], 2 * r - q[1]]; }));
  for (i = 1; i <= 3; i++) P.push([r + L2 * i / 3, 0]);
  var N = P.map(function (p, i) { var a = P[Math.max(0, i - 1)], c = P[Math.min(P.length - 1, i + 1)], tx = c[0] - a[0], tz = c[1] - a[1], l = Math.hypot(tx, tz); return [-tz / l, tx / l]; });
  var h = t / 2, IN = P.map(function (p, i) { return [p[0] + N[i][0] * h, p[1] + N[i][1] * h]; }), OUT = P.map(function (p, i) { return [p[0] - N[i][0] * h, p[1] - N[i][1] * h]; });
  /* inner surface: runs of segments with the same shade merged into one face (no seams) */
  var runs = [], cur = null;
  for (i = 0; i < P.length - 1; i++) {
    var sh = (N[i][1] + N[i + 1][1]) > (N[i][0] + N[i + 1][0]) ? 'top' : 'right';
    if (!cur || cur.sh !== sh) { cur = { sh: sh, idx: [i] }; runs.push(cur); }
    cur.idx.push(i + 1);
  }
  runs.forEach(function (r) {
    var f = r.idx.map(function (j) { return [IN[j][0], 0, IN[j][1]]; }).concat(r.idx.slice().reverse().map(function (j) { return [IN[j][0], d, IN[j][1]]; }));
    b.F(f, fam, r.sh, { ns: true });
  });
  var k = P.length - 1;
  b.F([[IN[0][0], 0, IN[0][1]], [OUT[0][0], 0, OUT[0][1]], [OUT[0][0], d, OUT[0][1]], [IN[0][0], d, IN[0][1]]], pfam, 'top');
  b.F([[IN[k][0], 0, IN[k][1]], [OUT[k][0], 0, OUT[k][1]], [OUT[k][0], d, OUT[k][1]], [IN[k][0], d, IN[k][1]]], fam, 'right');
  b.F(OUT.map(function (q) { return [q[0], d, q[1]]; }).concat(IN.slice().reverse().map(function (q) { return [q[0], d, q[1]]; })), fam, 'left');
  b.L(IN.map(function (q) { return [q[0], 0, q[1]]; }), 'outline', { w: 1 });
  return P;
}
G.sweep = function (b) { elbow(b, 12, 6, 6, 6, 6.5, 'n', 'm'); };
G.loft = function (b) {
  var c = [5, 5, 10], r = 3.6;
  b.L([[0, 0, 0], [10, 0, 0]], 'hid'); b.L([[0, 0, 0], [0, 10, 0]], 'hid');
  b.F([[0, 10, 0], [10, 10, 0]].concat(A3(c, r, 'z', 45, 135, 12)), 'n', 'left');
  b.F([[10, 0, 0], [10, 10, 0]].concat(A3(c, r, 'z', 45, -45, 12)), 'n', 'right');
  b.F(A3(c, r, 'z', 0, 360, 32).slice(0, -1), 'm', 'top');
  b.L([[0, 10, 0], [10, 10, 0], [10, 0, 0]], 'sk', { w: 1.8 });
};
G.draft = function (b) {
  b.F([[0, 0, 9], [8.5, 0, 9], [8.5, 12, 9], [0, 12, 9]], 'b', 'top');
  b.F([[12, 0, 0], [12, 12, 0], [8.5, 12, 9], [8.5, 0, 9]], 'm', 'right');
  b.F([[0, 12, 0], [12, 12, 0], [8.5, 12, 9], [0, 12, 9]], 'b', 'left');
  b.L([[12, 12, 0], [12, 12, 9.5]], 'hid', { detail: true });
  b.L(A3([12, 12, 0], 6.5, 'y', 90, 111, 8), 'con', { detail: true });
};
G.scale = function (b) {
  b.box(0, 0, 0, 5, 5, 5, 'b');
  b.edges(0, 0, 0, 12, 12, 12, 'ghost', false);
  b.L([[5.2, 5.2, 5.2], [11.5, 11.5, 11.5]], 'mod', { head: 1 });
};
G.thread = function (b) {
  var r = 3.6, h = 11;
  b.cyl(0, 0, 0, h, r, 'b', { noTop: true });
  for (var z = 0.6; z < h - 0.8; z += 2.1) {
    var seg = [];
    for (var a = -45; a <= 135; a += 9) seg.push([r * Math.cos(a * D), r * Math.sin(a * D), z + (a + 45) / 360 * 2.1]);
    b.L(seg, 'mod', { w: 1.3 });
  }
  var hex = []; for (var i = 0; i < 6; i++) hex.push([6 * Math.cos((30 + i * 60) * D), 6 * Math.sin((30 + i * 60) * D)]);
  b.prism(hex, h, 3.4, 'b');
};
G.splitBody = function (b) {
  b.box(0, 0, 0, 12, 5, 8, 'b');
  b.F([[-1.5, 6, -1.5], [13.5, 6, -1.5], [13.5, 6, 10], [-1.5, 6, 10]], 'p', 'left');
  b.box(0, 7, 0, 12, 5, 8, 'b', { fams: { left: 'b', top: 'b', right: 'b' } });
};
/* Split face (v2): only the face and the split line; one region blue. No body, so it can't be confused with Split body. */
G.splitFace = function (b) {
  b.F([[0, 0, 0], [14, 0, 0], [14, 14, 0], [0, 14, 0]], 'b', 'top');
  b.F([[7, 0, 0], [14, 0, 0], [14, 14, 0], [7, 14, 0]], 'm', 'top');
  b.L([[7, -2.5, 0], [7, 16.5, 0]], 'new', { w: 2.2 });
};
G.offsetPlane = function (b) {
  b.box(0, 0, 0, 12, 12, 3, 'b');
  b.L([[6, 6, 3.2], [6, 6, 13.4]], 'new', { head: 1 });
  b.F([[-1, -1, 14], [13, -1, 14], [13, 13, 14], [-1, 13, 14]], 'q', 'top');
};
G.planeAngle = function (b) {
  b.box(0, 0, 0, 12, 12, 3, 'b');
  b.F([[12, -1, 3], [12, 13, 3], [4, 13, 11], [4, -1, 11]], 'p', 'right');
  /* thin red angle arc from the top face up to the plane, cased so it reads over the body */
  var arc = A3([12, 13, 3], 9, 'y', 136, 179, 14);
  b.L(arc, 'gap', { w: 3.2 }); b.L(arc, 'con', { w: 1.3 });
};
G.intersectCurve = function (b) {
  var r = 5, h = 11;
  b.cyl(0, 0, 0, h, r, 'b');
  b.F([[-8, -8, 3], [8, -8, 3], [8, 8, 8], [-8, 8, 8]], 'q', 'top', { op: 0.5 });
  var curve = []; for (var a = -45; a <= 135; a += 6) curve.push([r * Math.cos(a * D), r * Math.sin(a * D), 5.5 + 2.5 * Math.sin(a * D)]);
  b.L(curve, 'new', { w: 1.6 });
};

G.intersectCurve16 = function (b) {
  var r = 5, h = 10;
  b.cyl(0, 0, 0, h, r, 'b');
  var curve = []; for (var a = -45; a <= 135; a += 6) curve.push([r * Math.cos(a * D), r * Math.sin(a * D), 5 + 2.6 * Math.sin(a * D)]);
  b.L(curve, 'new', { w: 1.9 });
};

/* ===== STATE BADGES ===== */
G.failed = function (b) { b.badge('err', 12, 12, 9); };
G.needsRepair = function (b) { b.f(CIRC(12, 12, 9, 32), 'y', 'left'); b.l(A2(12, 12, 4.6, 4.6, -60, 230, 20), 'white', { w: 2.2, head: 1 }); };
G.warning = function (b) { b.f([[12, 3], [21.5, 20], [2.5, 20]], 'y', 'left'); b.l([[12, 8.6], [12, 14.2]], 'ink', { w: 2.2, fixed: '#2b323c' }); b.m([12, 17], 'dotK', { r: 1.3 }); };
G.suppressed = function (b) { b.badge('off', 12, 12, 9); };
/* history strip: two live features | blue rollback marker | one rolled-back (dashed) feature */
G.rolledBack = function (b) {
  b.f([[2, 9.5], [6.5, 9.5], [6.5, 14.5], [2, 14.5]], 'b', 'left'); b.f([[7.8, 9.5], [12.3, 9.5], [12.3, 14.5], [7.8, 14.5]], 'b', 'left');
  b.f([[17.6, 9.5], [22, 9.5], [22, 14.5], [17.6, 14.5]], 'none', 'top');
  b.f([[14.2, 6], [15.8, 6], [15.8, 21], [14.2, 21]], 'm', 'right'); b.f([[12.4, 3], [17.6, 3], [15, 6.6]], 'm', 'left');
};
G.unsaved = function (b) { b.f([[5, 3], [14.5, 3], [19, 7.5], [19, 21], [5, 21]], 'w', 'top'); b.f([[14.5, 3], [14.5, 7.5], [19, 7.5]], 'w', 'right'); b.f(CIRC(17, 17, 5, 24), 'm', 'left'); };
G.readOnly = function (b) {
  b.f([[4, 3], [13.5, 3], [18, 7.5], [18, 21], [4, 21]], 'w', 'top'); b.f([[13.5, 3], [13.5, 7.5], [18, 7.5]], 'w', 'right');
  b.tu(A2(17, 15, 2.6, 2.6, 180, 360, 10).concat([[19.6, 17]]), 'k', 'top', 2, { prepend: [[14.4, 17]] });
  b.f([[12.8, 16.5], [21.2, 16.5], [21.2, 22], [12.8, 22]], 'k', 'left');
};

/* ===== APP CHROME (flat, 1.6 ink line + selective fills) ===== */
G.plus = function (b) { b.l([[12, 4.5], [12, 19.5]], 'ink', { w: 2.2 }); b.l([[4.5, 12], [19.5, 12]], 'ink', { w: 2.2 }); };
G.check = function (b) { b.l([[4.5, 12.5], [9.7, 17.7], [19.5, 6.5]], 'ink', { w: 2.4 }); };
G.close = function (b) { b.l([[5.5, 5.5], [18.5, 18.5]], 'ink', { w: 2.2 }); b.l([[18.5, 5.5], [5.5, 18.5]], 'ink', { w: 2.2 }); };
G.chevronDown = function (b) { b.l([[6, 9], [12, 15], [18, 9]], 'ink', { w: 2.2 }); };
G.menu = function (b) { [6.5, 12, 17.5].forEach(function (y) { b.l([[4.5, y], [19.5, y]], 'ink', { w: 2.2 }); }); };
G.overflow = function (b) { [5.5, 12, 18.5].forEach(function (x) { b.m([x, 12], 'dotI', { r: 2 }); }); };
G.search = function (b) { lens(b, null); };
G.settings = function (b) {
  /* filled cog in currentColor with a punched centre hole */
  b.f(GEAR(12, 12, 8, 9.8, 7.3, 0), 'ink', 'top', { holes: [CIRC(12, 12, 3.1, 24)] });
};
G.help = function (b) {
  b.l(A2(12, 12, 9, 9, 0, 360, 40), 'ink', { w: 2 });
  b.l(A2(12, 9.8, 3, 3, 195, 400, 14).concat([[12, 13.6]]), 'ink', { w: 2 }); b.m([12, 17.2], 'dotI', { r: 1.3 });
};
G.undo = function (b) { b.l([[4.5, 9]].concat(A2(13.5, 13.5, 6, 6, -90, 90, 14)).concat([[8, 19.5]]), 'ink', { w: 2, prepend: [[4.5, 9]], headStart: true }); };
G.redo = function (b) { b.l([[19.5, 9]].concat(A2(10.5, 13.5, 6, 6, 270, 90, 14)).concat([[16, 19.5]]), 'ink', { w: 2, headStart: true }); };
G.save = function (b) {
  b.l([[4, 4], [16.5, 4], [20, 7.5], [20, 20], [4, 20], [4, 4]], 'ink', { w: 2 });
  b.l([[8, 20], [8, 13.5], [16, 13.5], [16, 20]], 'ink', { w: 1.8 });
  b.l([[8, 4], [8, 8], [14, 8], [14, 4]], 'ink', { w: 1.8, detail: true });
};
G.import = function (b) { b.l([[3.5, 14], [3.5, 20], [20.5, 20], [20.5, 14]], 'ink', { w: 2 }); b.l([[12, 3], [12, 15]], 'ink', { head: 1, w: 2.2 }); };
G.export = function (b) { b.l([[3.5, 14], [3.5, 20], [20.5, 20], [20.5, 14]], 'ink', { w: 2 }); b.l([[12, 16], [12, 3.5]], 'ink', { head: 1, w: 2.2 }); };
G.delete = function (b) {
  b.l([[4, 6.5], [20, 6.5]], 'ink', { w: 2 }); b.l([[9, 6.5], [9.6, 3.5], [14.4, 3.5], [15, 6.5]], 'ink', { w: 1.9 });
  b.l([[6, 6.5], [7.2, 20.5], [16.8, 20.5], [18, 6.5]], 'ink', { w: 2 });
};
G.duplicate = function (b) {
  b.l([[8.5, 6], [8.5, 3.5], [20.5, 3.5], [20.5, 15.5], [18, 15.5]], 'ink', { w: 2 });
  var r = RR(3.5, 8.5, 12, 12, 1.5); r.push(r[0]); b.l(r, 'ink', { w: 2 });
};
G.edit = function (b) {
  b.l([[15.5, 4.5], [19.5, 8.5], [8.5, 19.5], [4.5, 19.5], [4.5, 15.5], [15.5, 4.5]], 'ink', { w: 2 });
  b.l([[13.2, 6.8], [17.2, 10.8]], 'ink', { w: 1.8 });
};
G.recent = function (b) { b.l(A2(12, 12, 8.6, 8.6, 0, 360, 40), 'ink', { w: 2 }); b.l([[12, 7], [12, 12], [15.5, 14.5]], 'ink', { w: 2 }); };
G.starred = function (b) { var s = []; for (var i = 0; i <= 10; i++) { var r = i % 2 ? 4.1 : 9.4, a = (-90 + i * 36) * D; s.push([12 + r * Math.cos(a), 12.8 + r * Math.sin(a)]); } b.l(s, 'ink', { w: 2 }); };
G.template = function (b) {
  var r = RR(3.5, 3.5, 17, 17, 2); r.push(r[0]);
  b.l(r, 'inkd', { w: 1.8 });
  b.l([[3.5, 8.5], [20.5, 8.5]], 'ink', { w: 2 });
};
G.extension = function (b) {
  var p = [].concat([[4, 8], [8.5, 8]], A2(10.5, 7, 2.2, 2.2, 150, 390, 14), [[12.5, 8], [17, 8], [17, 12]], A2(18, 14, 2.2, 2.2, 240, 480, 14), [[17, 16], [17, 20.5], [4, 20.5], [4, 8]]);
  b.l(p, 'ink', { w: 2 });
};
G.workspace = function (b) {
  var r = RR(3, 4, 18, 16, 2); r.push(r[0]);
  b.l(r, 'ink', { w: 2 });
  b.l([[9, 4.5], [9, 19.5]], 'ink', { w: 1.8 });
};
G.alert = function (b) { b.l([[12, 3.5], [21, 19.5], [3, 19.5], [12, 3.5]], 'ink', { w: 2 }); b.l([[12, 9], [12, 13.8]], 'ink', { w: 2 }); b.m([12, 16.6], 'dotI', { r: 1.2 }); };
G.tasks = function (b) {
  b.l([[3.5, 7.5], [5.7, 9.7], [9.5, 5.5]], 'ink', { w: 2.2 }); b.l([[12.5, 7.5], [20.5, 7.5]], 'ink', { w: 2 });
  b.l([[3.5, 16.5], [5.7, 18.7], [9.5, 14.5]], 'ink', { w: 2.2 }); b.l([[12.5, 16.5], [20.5, 16.5]], 'ink', { w: 2 });
};
G.study = function (b) {
  b.l([[3.5, 20.5], [20.5, 20.5]], 'ink', { w: 2 });
  b.f([[5.5, 12], [9, 12], [9, 19.5], [5.5, 19.5]], 'ink', 'top'); b.f([[10.3, 5], [13.8, 5], [13.8, 19.5], [10.3, 19.5]], 'ink', 'top'); b.f([[15.1, 9], [18.6, 9], [18.6, 19.5], [15.1, 19.5]], 'ink', 'top');
};
G.load = function (b) {
  b.l([[3, 19], [3, 5.5], [9.5, 5.5], [11.5, 7.5], [18.5, 7.5], [18.5, 11]], 'ink', { w: 2 });
  b.l([[3, 19], [6, 11], [21.5, 11], [18.5, 19], [3, 19]], 'ink', { w: 2 });
};
G.themeLight = function (b) {
  for (var i = 0; i < 8; i++) { var a = i * 45 * D; b.l([[12 + 7.4 * Math.cos(a), 12 + 7.4 * Math.sin(a)], [12 + 10 * Math.cos(a), 12 + 10 * Math.sin(a)]], 'ink', { w: 2 }); }
  b.l(A2(12, 12, 4.4, 4.4, 0, 360, 28), 'ink', { w: 2 });
};
/* Dark theme: a true crescent — outer circle minus an offset circle, built from the exact
   intersection points so both horns are clean and symmetric. */
G.themeDark = function (b) {
  var m = A2(12, 12, 8.5, 8.5, 8.9, 261.1, 30).concat(A2(16.5, 7.5, 7, 7, 213.8, 56.2, 22).slice(1)); m.push(m[0]);
  b.l(m, 'ink', { w: 2 });
};
G.themeSystem = function (b) {
  var r = RR(2.5, 4, 19, 12.5, 1.5); r.push(r[0]); b.l(r, 'ink', { w: 2 });
  b.f([[12, 4.8], [20.7, 4.8], [20.7, 15.7], [12, 15.7]], 'ink', 'top');
  b.l([[12, 16.5], [12, 20]], 'ink', { w: 2 }); b.l([[7.5, 20.5], [16.5, 20.5]], 'ink', { w: 2 });
};
/* ---------------------------------------------------------------- palette */
var PAL = {
  light: {
    outline: '#2b323c',
    b: ['#f2f4f7', '#cfd5de', '#aab3c0'], n: ['#ffdcae', '#f7a548', '#d97a16'], m: ['#b8d9ff', '#5ea3f7', '#2f78de'],
    r: ['#ffc9cb', '#ec5b60', '#c4383d'], gr: ['#c8efd3', '#4cc46b', '#2a9a4b'], y: ['#ffe39a', '#f6c343', '#d6a017'],
    k: ['#b6bec9', '#6c7684', '#4d5663'], w: ['#ffffff', '#f2f4f7', '#e3e7ec'], vf: ['#5ea3f7', '#5ea3f7', '#5ea3f7'],
    blue: '#2f7fe8', orange: '#e07f17', red: '#e5484d', green: '#2fa84f', amber: '#e0a412'
  },
  dark: {
    outline: '#0e1114',
    b: ['#aab3bf', '#7c8592', '#5d6672'], n: ['#ffd29a', '#f5a03e', '#c96f12'], m: ['#a6cfff', '#4f97f5', '#2c6fd0'],
    r: ['#ffa4a7', '#ec5b60', '#b8333a'], gr: ['#a3e6b8', '#3fb862', '#23853f'], y: ['#ffe08a', '#f0b92e', '#c08a0c'],
    k: ['#8f98a5', '#5a6370', '#434b56'], w: ['#eef0f3', '#cfd4db', '#b3bac4'], vf: ['#4f97f5', '#4f97f5', '#4f97f5'],
    blue: '#62a8ff', orange: '#ffa64d', red: '#ff6b70', green: '#4cc46b', amber: '#f0b92e'
  }
};
var SHI = { top: 0, left: 1, right: 2 };


function cadIconRender(props) {
  {
    var name = props.icon || 'extrude';
    var theme = props.theme === 'dark' ? 'dark' : 'light';
    var size = parseFloat(props.size) || 96;
    var bg = props.bg || (theme === 'dark' ? '#202327' : '#ffffff');
    var P = PAL[theme];
    var small = size <= 16;
    var b = Builder();
    /* 16 px masters: a simplified drawing replaces the 24 px one where it exists */
    /* compact = toolbar and tree sizes (<= 20 px): detail parts drop out and compact masters replace full drawings */
    var compact = size <= 20;
    (compact && G[name + '16'] ? G[name + '16'] : (G[name] || G.body))(b);
    var items = b.it.filter(function (i) { return !(compact && i.o.detail); });

    /* ---- project + fit 3D items ---- */
    var proj = function (p) { return [(p[0] - p[1]) * KX, (p[0] + p[1]) * KY - p[2]]; };
    var any3 = false, xs = [], ys = [];
    items.forEach(function (i) {
      if (!i.d3) return; any3 = true;
      i.pts = i.pts.map(proj); i.holes = (i.holes || []).map(function (h) { return h.map(proj); });
      if (i.o.prepend) i.o.prepend = i.o.prepend.map(proj);
      i.pts.forEach(function (p) { xs.push(p[0]); ys.push(p[1]); });
      i.holes.forEach(function (h) { h.forEach(function (p) { xs.push(p[0]); ys.push(p[1]); }); });
    });
    var fitScale = 1;
    if (any3) {
      var x0 = Math.min.apply(null, xs), x1 = Math.max.apply(null, xs), y0 = Math.min.apply(null, ys), y1 = Math.max.apply(null, ys);
      /* Compact sizes trade cross-icon scale consistency for legibility: the glyph fills
         a 21 px box instead of staying at the fixed scale U. */
      var isDefault = b.fit[0] === 2 && b.fit[1] === 2 && b.fit[2] === 22 && b.fit[3] === 22;
      var F = compact && isDefault ? [1.5, 1.5, 22.5, 22.5] : b.fit, bw = F[2] - F[0], bh = F[3] - F[1];
      fitScale = Math.min(compact ? 99 : U, bw / (x1 - x0), bh / (y1 - y0));
      var cx = (F[0] + F[2]) / 2, cy = (F[1] + F[3]) / 2, mx = (x0 + x1) / 2, my = (y0 + y1) / 2;
      var T = function (p) { return [cx + (p[0] - mx) * fitScale, cy + (p[1] - my) * fitScale]; };
      items.forEach(function (i) {
        if (!i.d3) return;
        i.pts = i.pts.map(T); i.holes = i.holes.map(function (h) { return h.map(T); });
        if (i.o.prepend) i.o.prepend = i.o.prepend.map(T);
      });
    }

    /* ---- weights (grid units) ---- */
    var px = 24 / size;
    var ow = Math.max(1, size / 32) * px;                         /* face outline: 1 px up to 32 px */
    var lw = size <= 16 ? 2.05 : size <= 20 ? 1.85 : size <= 24 ? 1.65 : 1.45;
    var fx = function (n) { return Math.round(n * 100) / 100; };
    var poly = function (pts, close) { return pts.map(function (p, i) { return (i ? 'L' : 'M') + fx(p[0]) + ' ' + fx(p[1]); }).join('') + (close ? 'Z' : ''); };
    var circ = function (p, r) { return 'M' + fx(p[0] - r) + ' ' + fx(p[1]) + 'a' + fx(r) + ' ' + fx(r) + ' 0 1 0 ' + fx(2 * r) + ' 0a' + fx(r) + ' ' + fx(r) + ' 0 1 0 ' + fx(-2 * r) + ' 0Z'; };

    var LC = {
      ink: ['currentColor', 1], outline: [P.outline, 1], thin: ['currentColor', 0.55], hid: ['currentColor', 0.6], none: ['none', 0],
      sk: [P.blue, 1], skc: [P.blue, 0.85], cons: [P.orange, 1], 'new': [P.orange, 1], mod: [P.blue, 1], con: [P.red, 1], cond: [P.red, 1],
      sel: [P.green, 1], axis: [P.blue, 1], ax: [P.red, 1], ay: [P.green, 1], az: [P.blue, 1], white: ['#ffffff', 1], ghost: [P.blue, 0.95],
      thinB: [P.m[2], 0.9], thinO: [P.outline, 0.6], inkd: ['currentColor', 1], dots: ['currentColor', 0.75], hatch: [P.outline, 0.55], amber: [P.amber, 1], gap: [bg, 1], tubeInk: [P.outline, 1]
    };
    var DASH = { dots: '0.01 3.4', inkd: '2.4 1.9', hid: '1.7 1.5', skc: '2 1.7', cons: '2.4 1.6', cond: '2 1.7', axis: '3.6 1.3 0.1 1.3', ghost: '1.6 1.4' };
    var WM = { dots: 1.15, thinO: 0.65, thin: 0.65, hid: 0.75, skc: 0.85, cons: 1, axis: 0.8, ghost: 0.8, thinB: 0.55, hatch: 0.5 };

    var S = [];
    var push = function (s) { S.push({ d: s.d || '', f: s.f || 'none', fo: s.fo == null ? 1 : s.fo, s: s.s || 'none', so: s.so == null ? 1 : s.so, w: s.w || 0, da: s.da || 'none' }); };
    var famFill = function (fam, sh) { var t = P[fam]; if (typeof t === 'string') return t; return t ? t[SHI[sh] == null ? 0 : SHI[sh]] : 'none'; };

    items.forEach(function (i) {
      var o = i.o;
      if (i.t === 'f') {
        var d = poly(i.pts, true) + i.holes.map(function (h) { return poly(h, true); }).join('');
        var st = { d: d, f: famFill(i.fam, i.sh), fo: o.op == null ? 1 : o.op, s: o.ns ? 'none' : P.outline, so: o.so == null ? 1 : o.so, w: ow };
        if (i.fam === 'q') { st.f = P.m[1]; st.fo = o.op == null ? 0.26 : o.op; st.s = P.m[2]; }
        if (i.fam === 'p') { st.f = P.n[1]; st.fo = o.op == null ? 0.42 : o.op; st.s = P.n[2]; }
        if (i.fam === 'ink') { st.f = 'currentColor'; st.s = 'none'; }
        if (i.fam === 'none') { st.f = 'none'; st.s = 'currentColor'; st.so = 0.7; st.da = '1.7 1.5'; st.w = lw * 0.75; }
        push(st);
      } else if (i.t === 'tube') {
        var wt = i.d3 ? i.w * fitScale : i.w;
        var dd = poly((o.prepend || []).concat(i.pts), false);
        push({ d: dd, s: P.outline, w: wt + 2 * ow });
        push({ d: dd, s: famFill(i.fam, i.sh), w: wt });
      } else if (i.t === 'l') {
        var lc = LC[i.k] || LC.ink, col = o.fixed || lc[0], w = o.w ? o.w * (small ? 1.12 : 1) : lw * (WM[i.k] || 1);
        if (i.k === 'tubeInk') { push({ d: poly(i.pts, false), s: P.outline, w: o.w }); return; }
        var p2 = (o.prepend || []).concat(i.pts.slice());
        var heads = [];
        var mk = function (atEnd) {
          var n = p2.length, E = atEnd ? p2[n - 1] : p2[0], j = atEnd ? n - 2 : 1, Sx = p2[j];
          while (Math.hypot(E[0] - Sx[0], E[1] - Sx[1]) < 1 && j > 0 && j < n - 1) { j += atEnd ? -1 : 1; Sx = p2[j]; }
          var ux = E[0] - Sx[0], uy = E[1] - Sx[1], L = Math.hypot(ux, uy) || 1; ux /= L; uy /= L;
          var h = Math.max(2.9, w * 2.05), hw = Math.max(1.95, w * 1.4);
          var bx = E[0] - ux * h, by = E[1] - uy * h;
          heads.push(poly([E, [bx - uy * hw, by + ux * hw], [bx + uy * hw, by - ux * hw]], true));
          var cut = [E[0] - ux * h * 0.7, E[1] - uy * h * 0.7];
          if (atEnd) p2[n - 1] = cut; else p2[0] = cut;
        };
        if (o.head) mk(true);
        if (o.head === 2 || o.headStart) mk(false);
        push({ d: poly(p2, false), s: col, so: (o.op == null ? lc[1] : o.op), w: w, da: DASH[i.k] || 'none' });
        heads.forEach(function (hd) { push({ d: hd, f: col, fo: (o.op == null ? lc[1] : o.op) }); });
      } else if (i.t === 'm') {
        var p = i.pts[0], k = i.k, r0 = small ? 0.3 : 0;
        if (k === 'pt') push({ d: circ(p, 1.6 + r0), f: theme === 'dark' ? '#eef0f3' : '#ffffff', s: P.outline, w: ow * 1.1 });
        else if (k === 'ptK') push({ d: circ(p, 1.6 + r0), f: 'currentColor' });
        else if (k === 'ptL') push({ d: circ(p, 2.5 + r0), f: theme === 'dark' ? '#eef0f3' : '#ffffff', s: P.outline, w: ow * 1.2 });
        else if (k === 'cpt') push({ d: circ(p, 2.2 + r0), f: P.r[1], s: P.outline, w: ow });
        else if (k === 'spt') push({ d: circ(p, (o.r || 1.35) + r0), f: P.blue });
        else if (k === 'dotK') push({ d: circ(p, o.r || 1.2), f: o.r && o.r < 1 ? '#2b323c' : '#2b323c' });
        else if (k === 'dotW') push({ d: circ(p, o.r || 1), f: '#ffffff' });
        else if (k === 'dotI') push({ d: circ(p, o.r || 1.2), f: 'currentColor' });
      }
    });

    var out = { size: size, label: name };
    for (var q = 0; q < NSLOT; q++) out['s' + q] = S[q] || { d: '', f: 'none', fo: 1, s: 'none', so: 1, w: 0, da: 'none' };
    out.slotCount = S.length;
    return out;
  }
}

function toSvg(o){var p='';for(var q=0;q<o.slotCount;q++){var s=o['s'+q];if(!s.d)continue;p+='<path d="'+s.d+'" fill="'+s.f+'" fill-opacity="'+s.fo+'" fill-rule="evenodd" stroke="'+s.s+'" stroke-opacity="'+s.so+'" stroke-width="'+s.w+'"'+(s.da!=='none'?' stroke-dasharray="'+s.da+'"':'')+'/>';}
return '<svg width="'+o.size+'" height="'+o.size+'" viewBox="0 0 24 24" fill="none" stroke-linejoin="round" stroke-linecap="round" style="display:block;overflow:visible" aria-hidden="true">'+p+'</svg>';}
var cache={};
function svgFor(name,theme,size,bg){var k=name+'|'+theme+'|'+size+'|'+bg;if(!cache[k])cache[k]=toSvg(cadIconRender({icon:name,theme:theme,size:size,bg:bg}));return cache[k];}
window.CadIcons={names:Object.keys(G),svg:svgFor};
if(!customElements.get('cad-icon')){
customElements.define('cad-icon',class extends HTMLElement{
static get observedAttributes(){return['name','size','theme','bg'];}
connectedCallback(){this.r();}
attributeChangedCallback(){this.r();}
r(){var s=parseFloat(this.getAttribute('size'))||20;this.style.display='inline-block';this.style.width=s+'px';this.style.height=s+'px';this.style.lineHeight='0';this.style.flex='none';
(this.shadowRoot||this.attachShadow({mode:'open'})).innerHTML='<style>:host{display:inline-block;flex:none;line-height:0;width:'+s+'px;height:'+s+'px}</style>'+svgFor(this.getAttribute('name')||'body',this.getAttribute('theme')||'light',s,this.getAttribute('bg')||'#ffffff');}
});}
})();
