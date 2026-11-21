// The app's panels and dialogs on a wide screen (each is a sheet or a
// screen of its own on a compact one), and the parts particular to them.

/// Share: the panel, and the link-access pill (pointer and touch).
abstract final class ShareSize {
  static const double width = 480, height = 640;
  static const double pill = 28, pillTouch = 36;
}

/// Version history: the panel, its list of versions (beside the preview,
/// or above it on a compact screen), a version row and the action bar.
abstract final class HistorySize {
  static const double width = 1100, height = 760;
  static const double list = 280, listCompact = 200, row = 48, actions = 56;
}

/// Triage: the proposals panel with its header and fact labels, and the
/// start dialog with a runner row.
abstract final class TriageSize {
  static const double width = 560, height = 600, header = 48, label = 76;
  static const double start = 380, runner = 56;
}

/// The import panel, the Ask AI panel, the update notice, and the AI
/// settings' runner menu.
abstract final class SmallPanels {
  static const double import = 480, askAi = 520, update = 340, runnerMenu = 220;
}

/// The schedule view: its widest column and a run row.
abstract final class ScheduleSize {
  static const double column = 640, run = 44, actions = 72;
}

/// The Today date strip: a day's height, the day's circle and its row of
/// dots, and a day's width (pointer and touch).
abstract final class DateStrip {
  static const double height = 82, day = 30, dotRow = 12, dot = 5;
  static const double extent = 58, extentTouch = 48;
}
