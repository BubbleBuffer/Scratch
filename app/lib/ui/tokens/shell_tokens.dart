// The app frame's own sizes: the shell around pages (sidebar, top bar,
// page stack, home, the bottom bar) and what floats over it.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

/// Where a toast floats: this far above the bottom of the window, clear of
/// a compact screen's bottom bar.
abstract final class ToastPlacement {
  static const double bottom = 80;
}

/// Home on a wide screen: its widest column and the margin either side of
/// it; the room under the last row, and on a compact screen the room that
/// keeps the last row clear of the new page button.
abstract final class HomeLayout {
  static const double width = 860, margin = 40;
  static const double endPadding = 64, compactEndPadding = 80;
}

/// The compact screen's bottom dock: a destination's width.
abstract final class MobileDock {
  static const double destination = 60;
}

/// The compact screen's page tree: a page's row (at least), an empty
/// folder's row, the indent per level, and the space tile with its glyph.
abstract final class MobileTree {
  static const double row = 52, emptyRow = 48, indent = 20;
  static const double tile = 26, tileGlyph = 18;
}

/// Where sheets sit on the desk (x from the rail's right edge).
abstract final class PileGeometry {
  /// The desk margin left of and above every sheet.
  static const margin = 12.0;

  /// The spine of the sheet directly below the top one.
  static const spine = 58.0;

  /// The top left corner of a sheet and its spine, and the icon tile on
  /// the spine.
  static const corner = Radius.circular(16);
  static const spineTile = 30.0;

  /// One older sheet's paper edge, and how many of them show at most.
  static const edge = 4.0;
  static const maxEdges = 3;

  /// The paper edges shown in a pile [depth] pages deep (Home not counted).
  static int edges(int depth) => depth < 2 ? 0 : math.min(depth - 2, maxEdges);

  /// The left of the spine in a pile [depth] deep (2 or more).
  static double spineLeft(int depth) => margin + edges(depth) * edge;

  /// The left of the top sheet in a pile [depth] deep.
  static double topLeft(int depth) =>
      depth <= 1 ? margin : spineLeft(depth) + spine;

  /// The left of sheet [index] (1 is the bottom one) in a pile [depth] deep.
  /// The sheet below the top one sits under the spine; older ones step back
  /// one paper edge each, down to the margin.
  static double sheetLeft(int index, int depth) {
    if (index >= depth) return topLeft(depth);
    final below = depth - 1 - index;
    return margin + math.max(0, edges(depth) - below) * edge;
  }
}

/// The dim over what a page or panel covers, at full strength: black at
/// [darkAlpha] in the dark theme, the ink at [lightAlpha] in the light one;
/// a compact screen's side panel dims what it slides over more
/// ([scrimDark], [scrimLight]).
abstract final class PageDim {
  static const dark = Color(0xFF000000);
  static const double darkAlpha = .16, lightAlpha = .035;
  static const double scrimDark = .4, scrimLight = .2;
}

/// The search overlay over a wide screen.
abstract final class SearchOverlay {
  static const double width = 640, height = 520;
}

/// The sidebar's page tree: the indent per level, the resize handle's grab
/// area, and where a dragged page's label floats from the pointer.
abstract final class SidebarTree {
  static const double indent = 12, resizeHandle = 6;
  static const ghostOffset = Offset(8, -14);
}
