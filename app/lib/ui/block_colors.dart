// Block colour palette (design/tokens.json `palette`): text
// colours, background colours and callout fills, light and dark. Colour
// names are the core's: `red` (text) and `red_background` (background);
// null or `default` means no colour.

import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// Fonts that carry emoji, after the text font (tests load Segoe UI Emoji).
const emojiFallback = [
  'Segoe UI Emoji',
  'Noto Color Emoji',
  'Apple Color Emoji',
];

/// Palette colour names in menu order (without `default`).
const paletteNames = [
  'gray',
  'brown',
  'orange',
  'yellow',
  'green',
  'blue',
  'purple',
  'pink',
  'red',
];

String paletteLabel(String name) => name == 'default'
    ? 'Default'
    : '${name[0].toUpperCase()}${name.substring(1)}';

Color _rgba(int r, int g, int b, double a) => Color.fromRGBO(r, g, b, a);

const _text = {
  'gray': Color(0xFF7D7A75),
  'brown': Color(0xFF9F765A),
  'orange': Color(0xFFD27B2D),
  'yellow': Color(0xFFCB9434),
  'green': Color(0xFF50946E),
  'blue': Color(0xFF387DC9),
  'purple': Color(0xFF9A6BB4),
  'pink': Color(0xFFC14C8A),
  'red': Color(0xFFCF5148),
};

final _bgLight = {
  'gray': _rgba(42, 28, 0, .07),
  'brown': _rgba(139, 46, 0, .086),
  'orange': _rgba(224, 101, 1, .13),
  'yellow': _rgba(211, 168, 0, .137),
  'green': _rgba(0, 100, 45, .09),
  'blue': _rgba(0, 124, 215, .094),
  'purple': _rgba(102, 0, 178, .08),
  'pink': _rgba(197, 0, 93, .086),
  'red': _rgba(223, 22, 0, .094),
};

final _bgDark = {
  'gray': _rgba(255, 255, 235, .1),
  'brown': _rgba(255, 177, 129, .192),
  'orange': _rgba(255, 147, 75, .28),
  'yellow': _rgba(255, 205, 75, .24),
  'green': _rgba(108, 255, 172, .157),
  'blue': _rgba(67, 155, 255, .24),
  'purple': _rgba(200, 125, 255, .2),
  'pink': _rgba(255, 103, 177, .23),
  'red': _rgba(255, 104, 92, .24),
};

const _calloutLight = {
  'default': Color(0xFFF9F8F7),
  'gray': Color(0xFFF0EFED),
  'brown': Color(0xFFF5EDE9),
  'orange': Color(0xFFFBEBDE),
  'yellow': Color(0xFFF9F3DC),
  'green': Color(0xFFE8F1EC),
  'blue': Color(0xFFE5F2FC),
  'purple': Color(0xFFF3EBF9),
  'pink': Color(0xFFFAE9F1),
  'red': Color(0xFFFCE9E7),
};

const _calloutDark = {
  'default': Color(0xFF202020),
  'gray': Color(0xFF383836),
  'brown': Color(0xFF45362D),
  'orange': Color(0xFF53361F),
  'yellow': Color(0xFF504425),
  'green': Color(0xFF263D30),
  'blue': Color(0xFF233850),
  'purple': Color(0xFF3C2D47),
  'pink': Color(0xFF4E2B3C),
  'red': Color(0xFF502C29),
};

/// True for `x_background` names.
bool isBackgroundColor(String? name) =>
    name != null && name.endsWith('_background');

String _base(String name) =>
    name.endsWith('_background') ? name.substring(0, name.length - 11) : name;

/// The text colour for [name] (`red`), or null for none or a background.
Color? paletteText(String? name, [AppColors? colors]) {
  if (name == null || isBackgroundColor(name)) return null;
  if (colors == null) return _text[name];
  return (colors.dark ? _readableDark : _readableLight)[name];
}

// Stored colour names and their swatches stay the same. Text variants blend
// toward the theme's ink only as far as needed for body-text AA, including
// highlights and callout surfaces on which that text can appear.
final _readableLight = {
  for (final entry in _text.entries)
    entry.key: _readableText(entry.value, AppColors.light),
};
final _readableDark = {
  for (final entry in _text.entries)
    entry.key: _readableText(entry.value, AppColors.darkTheme),
};

Color _readableText(Color color, AppColors c) {
  final surfaces = [
    c.bgPrimary,
    c.bgSecondary,
    c.desk,
    Color.alphaBlend(c.attnBg, c.bgPrimary),
    Color.alphaBlend(c.bgSelectedBlock, c.bgPrimary),
    for (final name in paletteNames) calloutFill(name, c),
    for (final name in paletteNames)
      Color.alphaBlend(paletteBackground(name, c)!, c.bgPrimary),
  ];
  double contrast(Color a, Color b) {
    final x = a.computeLuminance(), y = b.computeLuminance();
    return x > y ? (x + .05) / (y + .05) : (y + .05) / (x + .05);
  }

  for (var step = 0; step <= 64; step++) {
    final candidate = Color.lerp(color, c.textPrimary, step / 64)!;
    if (surfaces.every((surface) => contrast(candidate, surface) >= 4.5)) {
      return candidate;
    }
  }
  return c.textPrimary;
}

/// The background colour for [name] (`red_background`, or a bare `red`).
Color? paletteBackground(String? name, AppColors c) {
  if (name == null) return null;
  return (c.dark ? _bgDark : _bgLight)[_base(name)];
}

/// A swatch colour for menus: the text colour, or grey for default.
Color paletteSwatch(String name) =>
    _text[_base(name)] ?? const Color(0xFF7D7A75);

/// A callout's fill for its colour ([name] may be `red_background`, `red`
/// or null).
Color calloutFill(String? name, AppColors c) {
  final m = c.dark ? _calloutDark : _calloutLight;
  if (name == null || name == 'default') return c.bgSecondary;
  return m[_base(name)] ?? c.bgSecondary;
}
