// The Settings screen's family of controls (settings_kit.dart): its own
// colours from the mockup, its icon weight, the square switch, buttons,
// fields and panels, and the screen's frame.

import 'package:flutter/widgets.dart';

/// Colours the settings mockup uses that the palette has no role for.
abstract final class SettingsInk {
  /// The switch's "on" colour (a moss), light and dark.
  static const moss = Color(0xFF697A6B), mossDark = Color(0xFF8FA392);

  /// Text for destructive actions and errors, light and dark.
  static const danger = Color(0xFF8A3027), dangerDark = Color(0xFFF2B8B0);

  /// The left rule of a warning notice.
  static const warnRule = Color(0xFFC9A24C);
}

/// The settings icons' line weight.
abstract final class SettingsIcon {
  static const double stroke = 1.6;
}

/// The square switch, with a pointer and by touch: its track, knob and the
/// knob's travel, the knob's inset, and its hand-drawn uneven corners.
abstract final class SettingsSwitch {
  static const double width = 40, height = 22, knob = 16, travel = 18;
  static const double widthTouch = 46, heightTouch = 26, knobTouch = 20;
  static const double travelTouch = 20, inset = 2;
  static const knobCorners = BorderRadius.only(
    topLeft: Radius.circular(3),
    topRight: Radius.circular(4),
    bottomRight: Radius.circular(3),
    bottomLeft: Radius.circular(4),
  );
}

/// The settings controls' sizes.
abstract final class SettingsSize {
  /// A disabled row's fade.
  static const double disabledFade = .62, enabledFade = 1;

  /// The widest an intro, a description and a help note run.
  static const double intro = 560, description = 520, help = 540;

  /// The help "i" button, with a pointer and by touch.
  static const double helpButton = 22, helpButtonTouch = 28;

  /// A button and a select's least height, with a pointer and by touch; a
  /// field's.
  static const double control = 34, controlTouch = 40, field = 36;

  /// An icon button, with a pointer and by touch.
  static const double iconButton = 32, iconButtonTouch = 40;

  /// A select's least width.
  static const double select = 180;

  /// A choice card's radio, a status dot, a row's icon tile and a check
  /// row's status tile.
  static const double radio = 16, statusDot = 6, tile = 32, checkTile = 40;
}

/// The Settings screen's frame: the compact top bar, the sidebar of
/// sections, a section's widest column, a nav row's least height and its
/// badge, and the compact screen's list row.
abstract final class SettingsFrame {
  static const double topBar = 44, sidebar = 236, column = 720;
  static const double navRow = 32, badge = 18, listRow = 48;

  /// The room under a compact page's last row.
  static const double endPadding = 48;
}

/// Sync and its settings: a certificate fact's label column, the sync
/// details menu, the conflict log's tallest, and the pairing QR code
/// (default, compact and wider).
abstract final class SyncSize {
  static const double certLabel = 150, details = 300, conflictLog = 240;
  static const double qr = 184, qrCompact = 220, qrWide = 200;
}

/// The trash: its panel and popover, and a row with a path and without, on
/// a pointer and by touch.
abstract final class TrashSize {
  static const double width = 420, panel = 520, popover = 460;
  static const double row = 32, rowWithPath = 42, rowTouch = 56;
}

/// A QR code's modules and quiet zone: black on white whatever the theme,
/// so any camera reads it.
abstract final class QrInk {
  static const dark = Color(0xFF000000), light = Color(0xFFFFFFFF);
}
