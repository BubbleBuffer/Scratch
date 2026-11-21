// Colours that do not follow the theme: labels and handles drawn over a
// picture (a page cover, an image), and the page cover picker's swatches
// and gradients. Palettes with roles live in the bb_tokens palette; the
// block colours in block_color_tokens.dart.

import 'dart:ui' show Color;

/// A label or handle over a picture: a dark shade and white ink, the same
/// over any image in either theme.
abstract final class ImageOverlay {
  static const shade = Color(0x99000000);
  static const ink = Color(0xFFFFFFFF);
}

/// The page cover picker's colour covers.
abstract final class CoverColors {
  static const red = Color(0xFFE16259);
  static const yellow = Color(0xFFDFAB01);
  static const blue = Color(0xFF0B6E99);
  static const wood = Color(0xFFB3906F);
}

/// The page cover picker's gradient covers, in order.
abstract final class CoverGradients {
  static const one = [
    Color(0xFFFF4E9B),
    Color(0xFFFFE5B4),
    Color(0xFF9EF0D8),
    Color(0xFF7F9CF5),
  ];
  static const two = [Color(0xFF3B82F6), Color(0xFF9333EA)];
  static const three = [Color(0xFFF59E0B), Color(0xFFEF4444)];
  static const four = [Color(0xFF10B981), Color(0xFF3B82F6)];
  static const five = [Color(0xFFFDE68A), Color(0xFFFCA5A5)];
  static const six = [Color(0xFF111827), Color(0xFF6B7280)];
}
