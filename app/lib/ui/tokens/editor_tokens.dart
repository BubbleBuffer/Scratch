// The page editor's own measures: the rhythm of block rows, the hover
// handles, columns, headings' room above them, and the parts of particular
// blocks (callout, bookmark, todo, list markers, images).

import 'package:flutter/widgets.dart';

/// The room above and below a block's text: one line of body text is 28 px
/// (24 + 2 + 2 with a pointer, 22 + 3 + 3 by touch).
abstract final class BlockRhythm {
  static const double padPointer = 2, padTouch = 3;
  static double pad(bool touch) => touch ? padTouch : padPointer;

  /// The line the hover handles centre on.
  static const double handleLine = 24;

  /// A placeholder block's height before its body loads.
  static const double placeholder = 24;

  /// The page's text at the smaller "small text" size.
  static const double smallTextScale = 14 / 16;
}

/// The hover handles left of a block (add and grip).
abstract final class BlockHandle {
  static const double width = 18, height = 24;
}

/// Columns: the room between them, and left of each for its handles.
abstract final class BlockColumns {
  static const double gap = 46, gutter = 48;
}

/// The room above a heading, with a pointer and by touch.
abstract final class HeadingSpace {
  static const double h1 = 32, h1Touch = 20;
  static const double h2 = 22, h2Touch = 16;
  static const double h3 = 16, h3Touch = 12;

  /// A heading's task progress bar.
  static const double progressBar = 6;
}

/// List markers: the marker column, the bullet's box and its three shapes,
/// and a to-do's checkbox and the gap after it.
abstract final class ListMarker {
  static const double column = 24, bullet = 8;
  static const double disc = 3, ring = 2.75, ringStroke = 1.2, square = 5.5;
  static const double todoBox = 22, todoGap = 8;
}

/// A callout's emoji button and the bar of a plain callout in a space
/// colour.
abstract final class CalloutParts {
  static const double icon = 32, emoji = 17, barInset = 14;
}

/// A bookmark card and its favicon.
abstract final class BookmarkCard {
  static const double minHeight = 72, favicon = 14;

  /// How far below the row's top the card's handles sit.
  static const double handleDrop = 12;
}

/// A code block's language label above its box.
abstract final class CodeBlockParts {
  static const double label = 24;
}

/// The divider block's row, and its handles' offset from it.
abstract final class DividerBlock {
  static const double row = 13, handleTop = -5.5;
}

/// Media blocks: the empty "Add a ..." row; an image's least width, its
/// loading box and drop zone, and its resize grip.
abstract final class MediaBlock {
  static const double emptyRow = 46, imageMin = 48;

  /// An image's stored width is a percentage of the page's text column.
  static const double fullWidth = 100;
  static const double loading = 160, dropZone = 120;
  static const double grip = 6, gripHeight = 48, gripZone = 18;
  static const gripRadius = Radius.circular(20);
  static const gripShade = Color(0x99000000);
}

/// A page link's icon, and the small link badge on it.
abstract final class PageLinkParts {
  static const double row = 28, icon = 24, emoji = 17, badge = 10;
}

/// The table of contents: a row, and the indent per heading level.
abstract final class TocParts {
  static const double row = 30, indent = 24;
}

/// [style] at the page's "small text" size.
TextStyle smallTextStyle(TextStyle style) =>
    style.copyWith(fontSize: style.fontSize! * BlockRhythm.smallTextScale);

/// The editor's toolbars: the selection toolbar over text (a pointer), and
/// a button of the keyboard toolbar above the soft keyboard (touch).
abstract final class EditorToolbar {
  static const double selection = 36, keyButton = 40;
}

/// The editor's menus: the slash and mention menus beside the caret, and
/// the colour menu with its swatches (pointer, and by touch or in a sheet).
abstract final class EditorMenu {
  static const double width = 324, slashHeight = 400, colorHeight = 420;

  /// A menu below the caret moves above it when less room than this is
  /// left under the caret.
  static const double flipBelow = 240;
  static const double swatch = 22, swatchTouch = 26;
}

/// The page around the blocks: the band of the page's space colour along
/// the top, the compact select mode's action bar, and the room under the
/// last block that a tap puts a new block into (embedded, compact, and a
/// share of a wider page's height).
abstract final class PageChrome {
  static const double band = 6, selectBar = 52;
  static const double tapBelowEmbedded = 36, tapBelow = 160, tapBelowShare = .3;

  /// An embedded page's side gutter, compact and wider.
  static const double embeddedGutterCompact = 18, embeddedGutter = 30;

  /// A wide page's margin either side of its column, full width and not.
  static const double fullWidthMargin = 96, columnMargin = 48;
}

/// The `/` and mention menus above the soft keyboard: a header and rows of
/// [row], between [minRows] and [maxRows] of them, never shorter than
/// [floor] and clear of the window's top by [topClearance].
abstract final class SoftKeyboardMenu {
  static const double row = 44, header = 46, floor = 120, topClearance = 80;
  static const double minRows = 3, maxRows = 5.5;

  /// The rows kept in view below the caret when a menu opens.
  static const double keepRows = 4;

  static const double minHeight = minRows * row + header;
  static const double maxHeight = maxRows * row + header;
}
