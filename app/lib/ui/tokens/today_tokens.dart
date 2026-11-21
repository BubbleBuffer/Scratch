// Today: the week strip, the agenda's check column, the day's timeline, the
// done pile and the motion rings.

/// A day in the week strip: its height, and a dot per planned item.
abstract final class WeekStripSize {
  static const double day = 64, dotRow = 6, dot = 5, dotRadius = 1;
}

/// An agenda row: the check's hit area and the checkbox in it; the widest
/// a row's metadata runs, compact and wider.
abstract final class AgendaRow {
  static const double checkTarget = 28, check = 22;
  static const double metadataCompact = 174, metadata = 260;
}

/// The day's timeline. Positions are from the top of the timeline.
abstract final class DayTimeline {
  /// Its height with one lane of labels, and with a countdown line; each
  /// further lane adds [lane].
  static const double height = 86, countdownHeight = 100, lane = 53;

  /// The track: its top and thickness.
  static const double trackTop = 25, track = 8;

  /// An event's node: its top, its size, and the least distance between two
  /// nodes and from either end.
  static const double nodeTop = 17, node = 24, nodeGap = 28, nodeInset = 12;

  /// The room each event needs along a crowded day's track, which then
  /// scrolls.
  static const double perItem = 32;

  /// The first lane of labels' top.
  static const double labelTop = 47;

  /// The now marker: its label's top and its line; the label flips to the
  /// marker's left this close to the end, by [nowFlip].
  static const double nowTop = 4, nowLine = 40, nowLabelGap = 6;
  static const double nowFlipZone = 100, nowFlip = 95, nowLift = 4;

  /// The dot on the track at the current time.
  static const double nowDot = 8;

  /// Drawn on the node: its ring's inset, and a past event's dashes.
  static const double nodeRingInset = .75, dash = 6;
}

/// The done pile: its stack of slips and the cards fanned under the
/// agenda.
abstract final class DonePile {
  static const double width = 28, height = 24, slip = 7, slipStep = 5;
  static const double slipInset = 2, slipInsetOdd = 3;

  /// How much the pile grows when something lands on it.
  static const double bump = .15;

  /// The two cards peeking under the agenda: their insets and how far below
  /// it they reach.
  static const double backLeft = 22, backRight = 20, backDrop = 12;
  static const double frontInset = 10, frontDrop = 6, card = 18;
}

/// A page's progress ring in the motion list: its size, inset and stroke.
abstract final class MotionRing {
  static const double size = 44, inset = 3, stroke = 5;
}
