// A space's marks: the small square patch beside an item of the space, and
// the tile behind a page icon in the space's colours.

/// The patch beside an item that names its space.
abstract final class SpaceMark {
  /// Its side and its slightly softened corners (a patch, not a dot).
  static const double size = 8, radius = 2;
}

/// A page icon on its space's tile.
abstract final class SpaceTile {
  /// The tile's side, and the share of it the glyph takes.
  static const double size = 22, glyphShare = .7;
}
