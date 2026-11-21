// The app's illustrations: the calico mark, the sleeping cat loaf and the
// confetti. Their colours and proportions are the approved prototype's and
// deliberately independent of the theme.

import 'package:flutter/widgets.dart';

/// The calico mark's fixed colours.
abstract final class CalicoInk {
  static const cream = Color(0xFFFFFAF1);
  static const outline = Color(0xFF282A2F);
  static const apricot = Color(0xFFDE945F);
  static const blush = Color(0xFFDDA39D);
}

/// Sizes of the illustrations.
abstract final class ArtSizes {
  /// The calico mark beside the app's name, and its smaller size in the
  /// sidebar and on the compact home.
  static const double calicoMark = 28, calicoMarkSmall = 26;

  /// The sleeping cat loaf, at the size its paths are drawn.
  static const double catLoafWidth = 200, catLoafHeight = 140;

  /// The cat loaf's ground line.
  static const catGround = Radius.circular(1.5);
}

/// A confetti piece's uneven corners, so the pieces look cut by hand.
abstract final class ConfettiCorners {
  static const topLeft = Radius.circular(2);
  static const topRight = Radius.circular(4);
  static const bottomLeft = Radius.circular(3);
  static const bottomRight = Radius.circular(1);
}
