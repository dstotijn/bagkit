"""Draw the Netherlands as a network of points from bagkit's own address list.

The addresses are grouped into a grid; each cell with enough addresses becomes a point at
their centre, sized by how many there are, and neighbouring points are joined by lines. The
outline of the country, its coast, lakes and islands, follows from where people live.

It writes two images: netherlands.svg with the network, and netherlands-flow.svg with packets
of data travelling along it, each on the shortest route between two places chosen by how many
addresses they have.

    bagkit convert lvbag-extract-nl.zip --shape addresses --format csv --output tmp/landing/adreslijst.csv
    python scripts/landing_map.py tmp/landing/adreslijst.csv web/public
"""
import csv
import heapq
import math
import pathlib
import random
import sys

CELL = 0.065  # degrees of latitude per grid cell, about 7 km
MIN_ADDRESSES = 25  # cells with fewer addresses are left out
WIDTH = 1000  # the SVG's width in user units
ROUTES = 90  # routes, each with one packet
SPEED = 70  # user units per second
TRAIL = (4, 10, 18, 28, 40, 55)  # lengths of the dashes that make up a packet and its trail


def main():
    source, target = sys.argv[1], pathlib.Path(sys.argv[2])
    # An equirectangular projection around the country's middle latitude is close enough at
    # this scale.
    scale = math.cos(math.radians(52.2))
    cells = {}
    with open(source, newline='', encoding='utf-8') as f:
        for row in csv.DictReader(f):
            if not row['breedtegraad']:
                continue
            x, y = float(row['lengtegraad']) * scale, -float(row['breedtegraad'])
            key = (math.floor(x / CELL), math.floor(y / CELL))
            cell = cells.setdefault(key, [0, 0.0, 0.0])
            cell[0] += 1
            cell[1] += x
            cell[2] += y
    points = {k: (sx / n, sy / n, n) for k, (n, sx, sy) in cells.items() if n >= MIN_ADDRESSES}

    xs = [p[0] for p in points.values()]
    ys = [p[1] for p in points.values()]
    margin = CELL
    x0, y0 = min(xs) - margin, min(ys) - margin
    factor = WIDTH / (max(xs) - x0 + margin)
    height = math.ceil((max(ys) - y0 + margin) * factor)

    def xy(p):
        return (p[0] - x0) * factor, (p[1] - y0) * factor

    def at(p):
        x, y = xy(p)
        return f'{x:.1f} {y:.1f}'

    # Join each point to its neighbours to the east, south and south-east, which triangulates
    # the grid without crossing lines.
    lines = []
    neighbours = {k: [] for k in points}
    for (i, j), p in sorted(points.items()):
        for di, dj in ((1, 0), (0, 1), (1, 1)):
            q = points.get((i + di, j + dj))
            if q:
                lines.append(f'M{at(p)}L{at(q)}')
                neighbours[(i, j)].append((i + di, j + dj))
                neighbours[(i + di, j + dj)].append((i, j))
    # Denser places get larger points: the radius grows with the log of the addresses.
    dots = []
    for p in sorted(points.values(), key=lambda p: p[2]):
        r = 1.5 + 1.1 * math.log10(p[2] / MIN_ADDRESSES)
        x, y = at(p).split()
        dots.append(f'<circle cx="{x}" cy="{y}" r="{r:.1f}"/>')

    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {height}">'
        f'<path d="{"".join(lines)}" fill="none" stroke="#000" stroke-width="0.8" stroke-opacity="0.5"/>'
        f'<g fill="#000">{"".join(dots)}</g>'
        '</svg>\n'
    )
    (target / 'netherlands.svg').write_text(svg, encoding='utf-8')
    print(f'netherlands.svg: {len(points)} points, {len(lines)} lines, {len(svg) / 1024:.0f} KiB')

    # Packets: each route runs between two places drawn by their number of addresses, at least
    # a fifth of the country apart. A packet travels its route in the first half of its cycle
    # and waits out the second half; negative delays spread the starts. Its trail is several
    # faint dashes of growing length that share the packet's head, so where they overlap near
    # the head it is bright, and it fades towards the tail.
    rng = random.Random(2026)
    keys = sorted(points)
    weights = [points[k][2] for k in keys]
    defs, uses = [], []
    while len(defs) < ROUTES:
        a, b = rng.choices(keys, weights, k=2)
        if math.dist(xy(points[a]), xy(points[b])) < WIDTH / 5:
            continue
        route = shortest_route(neighbours, a, b, lambda k: xy(points[k]))
        if route is None:
            continue
        length = sum(math.dist(xy(points[p]), xy(points[q])) for p, q in zip(route, route[1:]))
        cycle = 2 * length / SPEED
        n = len(defs)
        # Each route's length is normalised to 100, so one animation fits all of them: the
        # pattern repeats every 200, and each dash ends at the pattern's start, the packet's head.
        defs.append(f'<path id="r{n}" pathLength="100" d="M{"L".join(at(points[k]) for k in route)}"/>')
        style = f'animation-duration:{cycle:.1f}s;animation-delay:-{rng.uniform(0, cycle):.1f}s'
        dashes = ''.join(
            f'<use href="#r{n}" stroke-dasharray="0 {200 - d:.2f} {d:.2f} 0"{" class=\"h\"" if i == 0 else ""}/>'
            for i, d in enumerate(t * 100 / length for t in TRAIL)
        )
        uses.append(f'<g style="{style}">{dashes}</g>')
    flow = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {height}">'
        '<style>g{animation:flow linear infinite}'
        'use{fill:none;stroke:#000;stroke-width:2.6;stroke-opacity:0.3}.h{stroke-linecap:round}'
        '@keyframes flow{from{stroke-dashoffset:200}to{stroke-dashoffset:0}}'
        '@media (prefers-reduced-motion:reduce){g{display:none}}</style>'
        f'<defs>{"".join(defs)}</defs>{"".join(uses)}'
        '</svg>\n'
    )
    (target / 'netherlands-flow.svg').write_text(flow, encoding='utf-8')
    print(f'netherlands-flow.svg: {len(defs)} routes, {len(flow) / 1024:.0f} KiB')


def shortest_route(neighbours, start, end, position):
    """The shortest route between two points along the network's lines, or None."""
    distance, previous = {start: 0.0}, {}
    queue = [(0.0, start)]
    while queue:
        d, k = heapq.heappop(queue)
        if k == end:
            route = [end]
            while route[-1] != start:
                route.append(previous[route[-1]])
            return route[::-1]
        if d > distance[k]:
            continue
        for n in neighbours[k]:
            nd = d + math.dist(position(k), position(n))
            if nd < distance.get(n, math.inf):
                distance[n], previous[n] = nd, k
                heapq.heappush(queue, (nd, n))
    return None


if __name__ == '__main__':
    main()
