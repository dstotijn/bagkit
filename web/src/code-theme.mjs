// The syntax theme for code blocks, in the landing page's palette: white text on near-black,
// keywords and literals in the site's blue, strings and punctuation in light grey and comments in
// a grey that keeps 4.5:1 contrast. One dark theme serves both colour modes.
export default {
	name: 'bagkit',
	type: 'dark',
	colors: {
		'editor.background': '#0f0f0f',
		'editor.foreground': '#ebebeb',
	},
	tokenColors: [
		{
			scope: ['comment', 'punctuation.definition.comment'],
			settings: { foreground: '#858585' },
		},
		{
			scope: ['string', 'string.quoted', 'constant.other.symbol', 'markup.inline.raw'],
			settings: { foreground: '#b4b4b4' },
		},
		{
			scope: [
				'keyword',
				'keyword.control',
				'keyword.other',
				'storage',
				'storage.type',
				'storage.modifier',
				'constant.numeric',
				'constant.language',
			],
			settings: { foreground: '#6b9bff' },
		},
		{
			scope: [
				'entity.name.function',
				'support.function',
				'entity.name.command',
				'entity.name.type',
				'support.type',
				'entity.name.tag',
				'support.type.property-name',
				'meta.object-literal.key',
			],
			settings: { foreground: '#ffffff' },
		},
		{
			scope: ['punctuation', 'meta.brace', 'keyword.operator', 'variable.parameter'],
			settings: { foreground: '#b4b4b4' },
		},
	],
};
