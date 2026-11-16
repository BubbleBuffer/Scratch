// Sizes of the app's pickers: the emoji picker and the page cover picker.

/// The emoji picker.
abstract final class EmojiPickerSize {
  /// Its popover, and the share of a compact screen its sheet takes.
  static const double width = 408, height = 356, sheetShare = .6;

  /// The tab bar and the filter row above the grid.
  static const double tabBar = 40, filterRow = 42;

  /// A grid cell, the emoji in it, and the dice on the random button.
  static const double cell = 32, glyph = 22, dice = 16;
}
