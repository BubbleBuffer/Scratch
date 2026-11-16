// Text that stands in for an icon: a block type's letter mark ("H1", "Aa")
// fills the icon box it replaces, so it is sized against that box rather
// than taken from the type scale.

import 'package:flutter/widgets.dart';

import '../tokens.dart';

/// Shares of the icon box a letter mark takes: its size, a toggle
/// heading's smaller size beside the triangle, and the line height.
abstract final class LetterMark {
  static const double size = .6, toggleSize = .5, line = .7;

  /// The triangle before a toggle heading's level.
  static const double triangle = .45;
}

/// The style of a letter mark in an icon box of [box] logical pixels.
TextStyle letterMarkStyle(double box, Color color, {bool toggle = false}) {
  final size = box * (toggle ? LetterMark.toggleSize : LetterMark.size);
  return TextStyle(
    inherit: false,
    fontFamily: sansFamily,
    fontFamilyFallback: [monoFamily],
    fontSize: size,
    height: box * LetterMark.line / size,
    leadingDistribution: TextLeadingDistribution.even,
    fontWeight: Weight.semibold.fontWeight,
    color: color,
    decoration: TextDecoration.none,
  );
}

/// The share of its icon box an emoji icon fills.
const double emojiIconShare = .85;

/// An emoji at [size] logical pixels, as a picker shows it.
TextStyle emojiStyle(double size, List<String> fallback) =>
    TextStyle(fontSize: size, height: 1, fontFamilyFallback: fallback);

/// The style of an emoji drawn as an icon in a box of [box] logical pixels.
TextStyle emojiIconStyle(double box, List<String> fallback) => TextStyle(
  fontSize: box * emojiIconShare,
  height: 1,
  fontFamilyFallback: fallback,
);
