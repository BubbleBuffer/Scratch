// Scratch's agent screens around the kit's chat: the dock and the full
// screen, the transcript's row gap, the user message bubble, and the
// sheets for references and "Add to note".

abstract final class AgentLayout {
  /// The full screen's list of chats, and the dock's chat menu.
  static const double sessions = 248, sessionMenu = 280;

  /// A chat row's menu.
  static const double rowMenu = 200;

  /// The dock's resize grip.
  static const double resizeGrip = 6;

  /// The chat column: the transcript's width plus its side room.
  static const double columnRoom = 64;

  /// The gap between transcript rows.
  static const double rowGap = 14;

  /// A notice in the middle of an empty or signed-out chat.
  static const double notice = 460;

  /// A user message's widest bubble.
  static const double userBubble = 560;
}

/// The sheets of the agent's references.
abstract final class AgentSheets {
  /// The reference picker, and "Add to note" with its preview.
  static const double picker = 480, addToNote = 520;
  static const double previewHeight = 76, previewWidth = 420;

  /// A markdown list's marker column.
  static const double listMarker = 22;
}
