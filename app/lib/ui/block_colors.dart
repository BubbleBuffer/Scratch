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

/// True for `x_background` names.
bool isBackgroundColor(String? name) =>
    name != null && name.endsWith('_background');

String _base(String name) =>
    name.endsWith('_background') ? name.substring(0, name.length - 11) : name;

/// The text colour for [name] (`red`), or null for none or a background.
Color? paletteText(String? name, [AppColors? colors]) {
  if (name == null || isBackgroundColor(name)) return null;
  if (colors == null) return BlockPalette.text[name];
  return (colors.dark ? _readableDark : _readableLight)[name];
}

// Stored colour names and their swatches stay the same. Text variants blend
// toward the theme's ink only as far as needed for body-text AA, including
// highlights and callout surfaces on which that text can appear.
final _readableLight = {
  for (final entry in BlockPalette.text.entries)
    entry.key: _readableText(entry.value, AppColors.light),
};
final _readableDark = {
  for (final entry in BlockPalette.text.entries)
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
  return (c.dark ? BlockPalette.backgroundDark : BlockPalette.backgroundLight)[_base(name)];
}

/// A swatch colour for menus: the text colour, or grey for default.
Color paletteSwatch(String name) =>
    BlockPalette.text[_base(name)] ?? BlockPalette.text['gray']!;

/// A callout's fill for its colour ([name] may be `red_background`, `red`
/// or null).
Color calloutFill(String? name, AppColors c) {
  final m = c.dark ? BlockPalette.calloutDark : BlockPalette.calloutLight;
  if (name == null || name == 'default') return c.bgSecondary;
  return m[_base(name)] ?? c.bgSecondary;
}
