// The editor's inline marks: bold, italic, links, mentions and inline code
// inside a block's text, and the chips drawn over dates and search queries.
// Their styles derive from the block's own style (already on the type
// scale), so these helpers scale it rather than pick a role.

import 'package:flutter/widgets.dart';

import '../tokens.dart';

abstract final class InlineMark {
  /// Inline code against the text around it.
  static const double codeScale = .85;

  /// A date chip's label: its size against the text, and its line height.
  static const double dateScale = .8, dateLine = 1.25;

  /// A search query chip's label (the mockup's 15 px on 16 px text).
  static const double queryScale = .94, queryLine = 1.4;

  /// The style assumed when a block's text has none.
  static const fallback = TextStyle(fontSize: 16);
  static const double fallbackLine = 1.5;
}

/// The style of a run with inline marks over text styled [base].
TextStyle inlineMarkStyle(
  TextStyle? base, {
  required bool bold,
  required bool italic,
  required bool mention,
  required bool code,
  required bool underlined,
  TextDecoration? decoration,
  Color? decorationColor,
  Color? color,
}) => TextStyle(
  // Two UI weights: a bold mark is strong; a mention keeps the text's
  // weight and stands out by its underline and icon.
  fontWeight: bold ? Weight.strong.fontWeight : null,
  fontStyle: italic ? FontStyle.italic : null,
  decoration: decoration,
  decorationColor: decorationColor,
  decorationThickness: underlined ? Stroke.hair : null,
  color: color,
  fontFamily: code ? monoFamily : null,
  fontFamilyFallback: code ? monoFallback : null,
  fontSize: code && base?.fontSize != null
      ? base!.fontSize! * InlineMark.codeScale
      : null,
);

/// [run] with [spacing] between its letters: the room a page mention's
/// icon takes in the text.
TextStyle spacedRun(TextStyle run, double spacing) =>
    run.copyWith(letterSpacing: spacing);

/// [base] drawn invisibly so a chip can be painted over it: [letterSpacing]
/// and [fontSize] stretch or shrink it to the chip's width.
TextStyle coveredRun(
  TextStyle base, {
  double? letterSpacing,
  double? fontSize,
  double? height,
}) => base.copyWith(
  color: kTransparent,
  decoration: TextDecoration.none,
  letterSpacing: letterSpacing,
  fontSize: fontSize,
  height: height,
);

/// A search query chip's label.
TextStyle queryLabelStyle(TextStyle base) => base.copyWith(
  fontSize: (base.fontSize ?? InlineMark.fallback.fontSize!) * InlineMark.queryScale,
  height: InlineMark.queryLine,
  fontWeight: Weight.strong.fontWeight,
  decoration: TextDecoration.none,
);

/// A query chip label's leading count.
TextStyle queryCountStyle(TextStyle label, Color color) =>
    label.copyWith(color: color, fontWeight: Weight.strong.fontWeight);

/// A date chip's label.
TextStyle dateChipStyle(TextStyle base, Color color) => base.copyWith(
  fontSize: (base.fontSize ?? InlineMark.fallback.fontSize!) * InlineMark.dateScale,
  height: InlineMark.dateLine,
  color: color,
  decoration: TextDecoration.none,
  fontWeight: Weight.strong.fontWeight,
);

/// [run] at [weight]: a bold mark, or a table's header row, over text whose
/// size comes from elsewhere.
TextStyle weightedRun(TextStyle run, Weight weight) =>
    run.copyWith(fontWeight: weight.fontWeight);
