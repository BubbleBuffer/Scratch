import 'dart:convert';

/// The `level` values of a chat search target, as the core writes them.
const chatSearchLevelChat = 'chat';
const chatSearchLevelChapter = 'chapter';
const chatSearchLevelSection = 'section';

/// Where a chat hit opens: the session and the message range, read from the
/// hit's `target` JSON. Titles are never used to find the chat.
///
/// This file only reads and writes the JSON. Opening a target (the agent
/// store, the page stack, toasts) is `chat_search.dart`.
class ChatSearchTarget {
  const ChatSearchTarget({
    required this.sessionId,
    required this.firstMessageId,
    required this.lastMessageId,
    required this.level,
    this.chapterTitle,
    this.sectionTitle,
  });

  final String sessionId;

  /// Empty for a title-only chat row, which has no message range.
  final String firstMessageId;
  final String lastMessageId;

  /// `chat`, `chapter` or `section`, as the core writes it.
  final String level;
  final String? chapterTitle;
  final String? sectionTitle;

  /// Whether the target has a message range to show (a title-only row does
  /// not, and opens its chat as an ordinary session).
  bool get anchored => firstMessageId.isNotEmpty && lastMessageId.isNotEmpty;

  /// The JSON the vault index stores for this target. Optional titles are
  /// left out when null.
  String encode() => jsonEncode({
    'sessionId': sessionId,
    'firstMessageId': firstMessageId,
    'lastMessageId': lastMessageId,
    'level': level,
    'chapterTitle': ?chapterTitle,
    'sectionTitle': ?sectionTitle,
  });

  /// Reads a hit's target; null when it is missing, has no session, or is not
  /// a chat target.
  static ChatSearchTarget? parse(String? target) {
    if (target == null || target.isEmpty) return null;
    try {
      final j = jsonDecode(target) as Map<String, Object?>;
      final sessionId = j['sessionId'];
      final first = j['firstMessageId'];
      final last = j['lastMessageId'];
      final level = j['level'];
      if (sessionId is! String ||
          sessionId.isEmpty ||
          first is! String ||
          last is! String ||
          level is! String) {
        return null;
      }
      return ChatSearchTarget(
        sessionId: sessionId,
        firstMessageId: first,
        lastMessageId: last,
        level: level,
        chapterTitle: j['chapterTitle'] as String?,
        sectionTitle: j['sectionTitle'] as String?,
      );
    } catch (_) {
      return null;
    }
  }
}
