// The Calendar screen (the tasks add-on): its panels, the continuous
// months, the week and day time grid, and the date jump.

abstract final class CalendarScreen {
  /// The keyboard shortcut list's key column and its longest list.
  static const double shortcutKey = 100, shortcutList = 360;

  /// The categories side panel, and the selected day's side panel.
  static const double categories = 225, dayPanel = 270;

  /// The compact screen's month above the day's agenda.
  static const double compactMonth = 310;

  /// The compact week strip's day.
  static const double weekDay = 62;

  /// The room under the agenda, and a dragged entry's width.
  static const double endPadding = 80, dragged = 170;

  /// A category's colour choice and its swatch.
  static const double colorChoice = 36, colorSwatch = 26;

  /// A task row's check target (compact screens get the full touch
  /// target), and the widest its source runs.
  static const double rowCheck = 26, rowCheckHeight = 24, rowSource = 160;

  /// A task row's details popover, and the dot of a task in it.
  static const double details = 250, detailsDot = 6;

  /// The menu of a note title's other dates.
  static const double datesMenu = 320;

  /// The quick task panel on a wide screen.
  static const double quickTaskWidth = 460, quickTaskHeight = 680;

  /// A task note's header: its tallest by touch and with a pointer.
  static const double noteHeaderTouch = 280, noteHeader = 340;

  /// The date jump's day row and its field.
  static const double jumpDay = 34, jumpField = 40;
}

/// The continuous months.
abstract final class CalendarMonth {
  /// A day's cell (compact, wider), its weekday header row, and the room
  /// under the last month.
  static const double cellCompact = 60, cell = 124, weekdays = 29, end = 40;

  /// A day's number row and the day's circle (compact, wider).
  static const double numberRowCompact = 39, numberRow = 32;
  static const double dayCompact = 30, dayWidth = 26, dayHeight = 24;

  /// An entry's chip, its checkbox, and a dot (an entry on a compact day,
  /// and the smaller one in the week strip).
  static const double chip = 22, chipCheck = 12, dot = 8, smallDot = 4;
}

/// The week and day time grid: the hour column, an hour's height, the
/// day's circle in the header, and the all-day row.
abstract final class CalendarTimeGrid {
  static const double hourColumn = 80, hour = 60, hours = 24 * hour;
  static const double headerDay = 27, allDayMin = 58, allDayMax = 107;

  /// An hour's label: its gap from the grid, and how it sits on the line
  /// (the first one below it).
  static const double labelRight = 9, labelLift = 7, firstLabelDrop = 2;

  /// The now label's lift onto the line, an empty all-day cell, and a
  /// timed block's inset from its lane.
  static const double nowLift = 8, emptyAllDay = 40, blockInset = 3;
}
