// Marks the first use of each abbreviation on a page with <abbr title="...">, so hovering it
// shows what it stands for. Only running text is marked: not code, links or headings.
import { defineHastPlugin } from 'satteri';

export const ABBREVIATIONS = {
	BAG: 'Basisregistratie Adressen en Gebouwen: the Dutch register of addresses and buildings',
	PDOK: "Publieke Dienstverlening Op de Kaart: the Dutch government's platform for geographic data",
	RD: 'Rijksdriehoeksmeting: the Dutch national coordinate system; RD New is EPSG:28992',
	WGS84: 'World Geodetic System 1984: the latitude and longitude GPS and web maps use, EPSG:4326',
	EPSG: 'A registry of coordinate systems, named after the European Petroleum Survey Group',
	CSV: 'Comma-separated values',
	XML: 'Extensible Markup Language',
	HTTP: 'Hypertext Transfer Protocol',
	JSON: 'JavaScript Object Notation',
	GDAL: 'Geospatial Data Abstraction Library',
	GIS: 'Geographic information system',
	CRS: 'Coordinate reference system',
	WKT: 'Well-known text: a text format for geometry',
	WKB: 'Well-known binary: a binary format for geometry',
	API: 'Application programming interface',
};

const pattern = new RegExp(`\\b(${Object.keys(ABBREVIATIONS).join('|')})\\b`, 'g');
// Elements whose text is left alone, along with everything inside them.
const skip = new Set(['code', 'pre', 'a', 'abbr', 'kbd', 'script', 'style']);

// Returns the children with the first use of each abbreviation not yet in `seen` marked.
function mark(children, seen) {
	let changed = false;
	const out = [];
	for (const child of children) {
		if (child.type === 'element' && !skip.has(child.tagName) && child.children) {
			const inner = mark(child.children, seen);
			if (inner) {
				out.push({ ...child, children: inner });
				changed = true;
			} else out.push(child);
			continue;
		}
		if (child.type !== 'text') {
			out.push(child);
			continue;
		}
		let last = 0;
		for (const match of child.value.matchAll(pattern)) {
			const word = match[1];
			if (seen.has(word)) continue;
			seen.add(word);
			changed = true;
			if (match.index > last)
				out.push({ type: 'text', value: child.value.slice(last, match.index) });
			out.push({
				type: 'element',
				tagName: 'abbr',
				properties: { title: ABBREVIATIONS[word] },
				children: [{ type: 'text', value: word }],
			});
			last = match.index + word.length;
		}
		if (last === 0) out.push(child);
		else if (last < child.value.length) out.push({ type: 'text', value: child.value.slice(last) });
	}
	return changed ? out : null;
}

export default defineHastPlugin({
	name: 'abbreviations',
	before(_root, ctx) {
		ctx.data.abbreviationsSeen = new Set();
	},
	element: {
		filter: ['p', 'li', 'td', 'th', 'dd'],
		visit(node, ctx) {
			const children = mark(node.children ?? [], ctx.data.abbreviationsSeen);
			if (children) ctx.replaceNode(node, { ...node, children });
		},
	},
});
