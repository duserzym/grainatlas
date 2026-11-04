"""Project Pupin-type zircon STL models (from duserzym/mineral-habit-matrix) into shaded SVG groups
and write grainatlas.svg and upbstudio.svg. Usage: python3 -I zircons.py <stl_dir>

The crystal shapes (zircon_<type>.stl) come from mineral-habit-matrix
(https://github.com/duserzym/mineral-habit-matrix), MIT licence, by Yiming Zhang.
grainatlas.svg becomes tauri/assets-icon.svg (`make icons` builds the app icon set from it) and,
with viewBox="36 36 440 440", ui/public/favicon.svg. Needs numpy."""
import struct, sys
from pathlib import Path
import numpy as np

STL = Path(sys.argv[1])
PALETTE = {"100": "#eab25a", "110": "#c8683a", "101": "#f6d29a", "211": "#dc9356", "301": "#d08a5c"}
GREY = {"100": "#c9d3df", "110": "#a9b6c5", "101": "#e3e9f0", "211": "#d2dbe5", "301": "#bfc9d5"}
JBLUE = {"100": "#7b93e6", "110": "#4063d8", "101": "#c3cff6", "211": "#9db0ee", "301": "#6a84e0"}
JGREEN = {"100": "#6fc45a", "110": "#389826", "101": "#c6ecb8", "211": "#95d682", "301": "#5cb848"}
JPURPLE = {"100": "#b98bcc", "110": "#9558b2", "101": "#e2cdea", "211": "#cba9d9", "301": "#a673c0"}
JRED = {"100": "#e2756d", "110": "#cb3c33", "101": "#f5c3bf", "211": "#ec9a94", "301": "#d8574f"}

def read_stl(p):
    b = p.read_bytes(); n = struct.unpack_from("<I", b, 80)[0]
    tri = np.frombuffer(b, dtype=np.dtype([("n", "<3f4"), ("v", "<9f4"), ("a", "<u2")]), count=n, offset=84)
    return tri["v"].reshape(-1, 3, 3).astype(float)

def faces_from_tris(tris):
    """Merge coplanar triangles into convex polygon faces; return list of (normal, loop points 3D)."""
    v = tris
    n = np.cross(v[:, 1] - v[:, 0], v[:, 2] - v[:, 0]); n /= np.linalg.norm(n, axis=1)[:, None]
    d = np.einsum("ij,ij->i", n, v[:, 0])
    key = [tuple(np.round(np.r_[n[i], d[i]], 4)) for i in range(len(v))]
    groups = {}
    for i, k in enumerate(key): groups.setdefault(k, []).append(i)
    out = []
    for k, idx in groups.items():
        nn = np.array(k[:3]); pts = np.unique(np.round(v[idx].reshape(-1, 3), 6), axis=0)
        # 2D hull in the face plane
        a = np.cross(nn, [0, 0, 1]); a = np.cross(nn, [1, 0, 0]) if np.linalg.norm(a) < 1e-6 else a
        a /= np.linalg.norm(a); b = np.cross(nn, a)
        uv = np.c_[pts @ a, pts @ b]; h = hull(uv)
        loop = [pts[np.argmin(np.linalg.norm(uv - q, axis=1))] for q in h]
        out.append((nn, np.array(loop)))
    return out

def hull(pts):
    pts = sorted(set(map(tuple, pts)))
    cross = lambda o, a, b: (a[0]-o[0])*(b[1]-o[1]) - (a[1]-o[1])*(b[0]-o[0])
    lo, up = [], []
    for p in pts:
        while len(lo) >= 2 and cross(lo[-2], lo[-1], p) <= 0: lo.pop()
        lo.append(p)
    for p in reversed(pts):
        while len(up) >= 2 and cross(up[-2], up[-1], p) <= 0: up.pop()
        up.append(p)
    return lo[:-1] + up[:-1]

def form(n):
    """Classify a tetragonal zircon face normal: prisms {100}/{110}, pyramids {101}/{211}/{301}."""
    az = np.degrees(np.arctan2(abs(n[1]), abs(n[0]))) % 90; az = min(az, 90 - az)
    pol = np.degrees(np.arccos(abs(n[2])))
    if pol > 85: return "110" if az > 30 else "100"
    if az > 12: return "211"
    return "301" if pol > 64 else "101"

def project(name, turn, tilt, size, centre, pal, edge, zoning=False, light=(-0.45, -0.8, 0.55), roll=0.0):
    tris = read_stl(STL / f"zircon_{name}.stl"); faces = faces_from_tris(tris)
    t, x = np.radians(turn), np.radians(tilt)
    rz = np.array([[np.cos(t), -np.sin(t), 0], [np.sin(t), np.cos(t), 0], [0, 0, 1]])
    rx = np.array([[1, 0, 0], [0, np.cos(x), -np.sin(x)], [0, np.sin(x), np.cos(x)]])
    R = rx @ rz; L = np.array(light) / np.linalg.norm(light)
    allv = np.vstack([f[1] for f in faces]) @ R.T
    xy = np.c_[allv[:, 0], -allv[:, 2]]; lo, hi = xy.min(0), xy.max(0)
    s = size / (hi - lo).max(); c = np.array(centre)
    rr = np.radians(roll); M = np.array([[np.cos(rr), -np.sin(rr)], [np.sin(rr), np.cos(rr)]])
    def P(v):
        V = v @ R.T; q = np.c_[V[:, 0], -V[:, 2]]; return ((q - (lo + hi) / 2) * s) @ M.T + c
    polys, edges = [], []
    for nn, loop in faces:
        n = R @ nn
        if n[1] >= -1e-9: continue
        shade = 0.66 + 0.48 * max(0.0, float(n @ L))
        base = np.array([int(pal[form(nn)][i:i+2], 16) for i in (1, 3, 5)], float)
        col = "#" + "".join(f"{int(np.clip(v, 0, 255)):02x}" for v in base * shade + 255 * 0.06)
        pts = " ".join(f"{a:.1f},{b:.1f}" for a, b in P(loop))
        polys.append(f'<polygon points="{pts}" fill="{col}"/>'); edges.append(f'<polygon points="{pts}"/>')
    H = np.array(hull(np.round(P(np.vstack([f[1] for f in faces])), 2)))
    hp = " ".join(f"{a:.1f},{b:.1f}" for a, b in H)
    g = [f'<polygon points="{hp}" fill="{pal["110"]}"/>'] + polys
    g.append(f'<g fill="none" stroke="{edge}" stroke-opacity="0.55" stroke-width="2.6" stroke-linejoin="round">' + "".join(edges) + "</g>")
    g.append(f'<polygon points="{hp}" fill="none" stroke="{edge}" stroke-width="4" stroke-linejoin="round" stroke-opacity="0.8"/>')
    if zoning:  # oscillatory CL zoning suggested by inset outlines of the silhouette
        cen = H.mean(0)
        for k, op in ((0.78, 0.55), (0.58, 0.45), (0.40, 0.5), (0.22, 0.45)):
            Z = cen + (H - cen) * k
            zp = " ".join(f"{a:.1f},{b:.1f}" for a, b in Z)
            g.append(f'<polygon points="{zp}" fill="none" stroke="#fff1dc" stroke-opacity="{op}" stroke-width="3" stroke-linejoin="round"/>')
    return "\n    ".join(g)

HEAD = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" role="img" aria-labelledby="title desc">
  <title id="title">{title}</title>
  <desc id="desc">{desc}</desc>
  <defs>
    <linearGradient id="tile" x2="0.8" y2="1"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#e9eef5"/></linearGradient>
    <linearGradient id="paper" x2="0.8" y2="1"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#f1f5f9"/></linearGradient>
    <linearGradient id="sem" x2="0.6" y2="1"><stop stop-color="#dfe6f7"/><stop offset="1" stop-color="#b9c6ec"/></linearGradient>
    <linearGradient id="sem2" x2="0.6" y2="1"><stop stop-color="#e9edf9"/><stop offset="1" stop-color="#c9d3f0"/></linearGradient>
    <linearGradient id="blue" x2="0.8" y2="1"><stop stop-color="#a8b8f2"/><stop offset="1" stop-color="#4063d8"/></linearGradient>
    <linearGradient id="orange" x2="0.8" y2="1"><stop stop-color="#efa39d"/><stop offset="1" stop-color="#cb3c33"/></linearGradient>
    <radialGradient id="spotglow" cx="0.5" cy="0.5" r="0.5"><stop stop-color="#7dd3fc" stop-opacity="0.9"/><stop offset="0.5" stop-color="#38bdf8" stop-opacity="0.35"/><stop offset="1" stop-color="#38bdf8" stop-opacity="0"/></radialGradient>
    <filter id="shadow" x="-25%" y="-25%" width="150%" height="150%"><feDropShadow dx="0" dy="9" stdDeviation="10" flood-color="#0f1b2d" flood-opacity="0.25"/></filter>
    <filter id="soft" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="1.6"/></filter>
    <clipPath id="mosaic"><rect x="96" y="96" width="320" height="320" rx="22"/></clipPath>
  </defs>
  <rect x="36" y="36" width="440" height="440" rx="88" fill="url(#tile)"/>
  <rect x="36" y="36" width="440" height="440" rx="88" fill="none" stroke="#d9e1ec" stroke-width="10"/>
'''

ga = HEAD.format(title="grainatlas icon", desc="Pupin-type zircon crystals from the Mineral Habit Matrix models (S13 in front; P3, L4 and S5 behind) on a stitched mosaic of four SEM tiles, with a laser spot and a scale bar, on a softly shaded tile.") + f'''  <g filter="url(#shadow)">
    <rect x="96" y="96" width="320" height="320" rx="22" fill="#b9c6ec"/>
    <g clip-path="url(#mosaic)">
      <rect x="90" y="90" width="172" height="172" fill="url(#sem)"/>
      <rect x="258" y="96" width="172" height="170" fill="url(#sem2)"/>
      <rect x="94" y="258" width="170" height="172" fill="url(#sem2)"/>
      <rect x="262" y="262" width="172" height="172" fill="url(#sem)"/>
      <path d="M260 90 V262 M90 262 H262 M262 262 V430 M258 262 H430" fill="none" stroke="#ffffff" stroke-opacity="0.9" stroke-width="3"/>
    </g>
    {project("P3", 20, 10, 92, (150, 158), JBLUE, "#24408f", roll=22)}
    {project("L4", -60, 12, 88, (368, 150), JGREEN, "#1f5f16", roll=-38)}
    {project("S5", 75, 8, 78, (364, 334), JPURPLE, "#5e3378", roll=62)}
    {project("AB3", 30, 12, 72, (146, 338), JRED, "#7f1f1a", roll=-18)}
    {project("S13", -28, 14, 236, (250, 258), PALETTE, "#5b2a12", roll=-30)}
    <circle cx="298" cy="214" r="38" fill="url(#spotglow)"/>
    <circle cx="298" cy="214" r="22" fill="none" stroke="#4063d8" stroke-width="5"/>
    <circle cx="298" cy="214" r="22" fill="none" stroke="#0ea5e9" stroke-width="2.5" filter="url(#soft)"/>
    <path d="M298 184 V194 M298 234 V244 M268 214 H278 M318 214 H328" stroke="#4063d8" stroke-width="4" stroke-linecap="round"/>
    <rect x="296" y="384" width="96" height="10" rx="3" fill="#1e293b"/>
    <rect x="296" y="376" width="4" height="26" rx="1.5" fill="#1e293b"/>
    <rect x="388" y="376" width="4" height="26" rx="1.5" fill="#1e293b"/>
  </g>
</svg>
'''
Path("grainatlas.svg").write_text(ga)

us = HEAD.format(title="UPb Studio icon", desc="A concordia diagram with age ticks, two error ellipses and a Pupin S2 zircon from the Mineral Habit Matrix models, on a softly shaded tile.") + f'''  <g filter="url(#shadow)">
    <rect x="96" y="96" width="320" height="320" rx="22" fill="url(#paper)" stroke="#cbd5e1" stroke-width="3"/>
    <path d="M160 128 V384 M224 128 V384 M288 128 V384 M352 128 V384 M128 160 H384 M128 224 H384 M128 288 H384 M128 352 H384" stroke="#e2e8f0" stroke-width="2"/>
    <path d="M128 128 V384 H384" fill="none" stroke="#334155" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M134 378 C 180 360, 230 322, 272 266 C 310 216, 342 170, 372 138" fill="none" stroke="#9558b2" stroke-width="9" stroke-linecap="round"/>
    <circle cx="176" cy="362" r="7" fill="#9558b2"/><circle cx="228" cy="324" r="7" fill="#9558b2"/><circle cx="272" cy="266" r="7" fill="#9558b2"/><circle cx="318" cy="206" r="7" fill="#9558b2"/><circle cx="358" cy="156" r="7" fill="#9558b2"/>
    <ellipse cx="248" cy="296" rx="44" ry="22" transform="rotate(-46 248 296)" fill="url(#blue)" fill-opacity="0.45" stroke="#24408f" stroke-width="4"/>
    <ellipse cx="330" cy="190" rx="36" ry="17" transform="rotate(-50 330 190)" fill="url(#orange)" fill-opacity="0.5" stroke="#8f2a24" stroke-width="4"/>
    {project("S2", -30, 14, 104, (180, 186), JGREEN, "#1f5f16", roll=-32)}
  </g>
</svg>
'''
Path("upbstudio.svg").write_text(us)
print("ok")
