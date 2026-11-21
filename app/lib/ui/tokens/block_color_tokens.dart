// The block colour palette's values (design/tokens.json `palette`): text
// colours, translucent backgrounds and opaque callout fills, light and dark,
// keyed by the core's colour names. block_colors.dart reads them.

import 'dart:ui' show Color;

abstract final class BlockPalette {
  /// Text colours.
  static const text = {
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

  /// Background colours over the light theme.
  static final backgroundLight = {
    'gray': Color.fromRGBO(42, 28, 0, .07),
    'brown': Color.fromRGBO(139, 46, 0, .086),
    'orange': Color.fromRGBO(224, 101, 1, .13),
    'yellow': Color.fromRGBO(211, 168, 0, .137),
    'green': Color.fromRGBO(0, 100, 45, .09),
    'blue': Color.fromRGBO(0, 124, 215, .094),
    'purple': Color.fromRGBO(102, 0, 178, .08),
    'pink': Color.fromRGBO(197, 0, 93, .086),
    'red': Color.fromRGBO(223, 22, 0, .094),
  };

  /// Background colours over the dark theme.
  static final backgroundDark = {
    'gray': Color.fromRGBO(255, 255, 235, .1),
    'brown': Color.fromRGBO(255, 177, 129, .192),
    'orange': Color.fromRGBO(255, 147, 75, .28),
    'yellow': Color.fromRGBO(255, 205, 75, .24),
    'green': Color.fromRGBO(108, 255, 172, .157),
    'blue': Color.fromRGBO(67, 155, 255, .24),
    'purple': Color.fromRGBO(200, 125, 255, .2),
    'pink': Color.fromRGBO(255, 103, 177, .23),
    'red': Color.fromRGBO(255, 104, 92, .24),
  };

  /// Opaque callout fills, light.
  static const calloutLight = {
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

  /// Opaque callout fills, dark.
  static const calloutDark = {
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
}
