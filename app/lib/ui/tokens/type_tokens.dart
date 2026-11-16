// Type helpers beyond the shared roles: the eyebrow label's tracking, and
// the height of one line of a style that came from elsewhere.

import 'package:flutter/widgets.dart';

import '../tokens.dart';

/// An eyebrow: a small upper-case label tracked out.
TextStyle eyebrowStyle(TypeSpec spec, Color color, {Weight? weight}) =>
    ts(spec, color, weight: weight).copyWith(letterSpacing: _eyebrowTracking);

const _eyebrowTracking = .5;

/// The height of one line of [style]; a style without a size or height is
/// taken as 14 px at 1.3.
double lineExtent(TextStyle style) =>
    (style.height ?? _fallbackLine) * (style.fontSize ?? _fallbackSize);

const _fallbackLine = 1.3, _fallbackSize = 14.0;
