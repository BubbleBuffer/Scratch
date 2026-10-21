import 'navigation.dart';
import '../../services/addon_services.dart';
import 'outline_names.dart';
// A note's reference to a chat's chapter or one of its sections. The link is
// the ids (session and the message range); the titles, summary and decision
// are a cache, shown when the agent cannot be reached. A reference is stored
// as JSON in the block's `target`; the block's text is the summary, so notes
// search finds what the chat concluded (raw chat text stays out of notes).

import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show OutlineChapter, OutlineSection;
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'chat_outline_display.dart';

/// Which level of the chat outline a reference points at.
enum ChatRefLevel { chapter, section }

/// Colour names for chapter and section bands, from the space family.
const chatRefBands = ['sun', 'moss', 'sky', 'paw', 'apricot'];

/// A stable band for [id]: the same chapter keeps its colour across runs.
String chatRefBandFor(String id) {
  var h = 0;
  for (final u in id.codeUnits) {
    h = (h * 31 + u) & 0x7fffffff;
  }
  return chatRefBands[h % chatRefBands.length];
}

/// One section inside a chapter's display list.
class ChatRefSection {
  const ChatRefSection({
    required this.id,
    required this.title,
    required this.firstMessageId,
    required this.lastMessageId,
    required this.colour,
    this.decision,
  });

  final String id;
  final String title;
  final String firstMessageId;
  final String lastMessageId;
  final String colour;
  final String? decision;

  Map<String, Object?> toJson() => {
    'id': id,
    'title': title,
    'firstMessageId': firstMessageId,
    'lastMessageId': lastMessageId,
    'colour': colour,
    if (decision != null) 'decision': decision,
  };

  factory ChatRefSection.fromJson(Map<String, Object?> j) => ChatRefSection(
    id: j['id'] as String,
    title: j['title'] as String,
    firstMessageId: j['firstMessageId'] as String,
    lastMessageId: j['lastMessageId'] as String,
    colour: j['colour'] as String? ?? 'sun',
    decision: j['decision'] as String?,
  );
}

/// A chapter or section of a chat, with its cached display fields.
class ChatReference {
  const ChatReference({
    required this.level,
    required this.sessionId,
    required this.firstMessageId,
    required this.lastMessageId,
    required this.chatTitle,
    required this.chapterTitle,
    required this.summary,
    this.sectionTitle,
    this.decision,
    this.when = '',
    this.colour = 'sun',
    this.sections = const [],
  });

  final ChatRefLevel level;
  final String sessionId;
  final String firstMessageId;
  final String lastMessageId;

  /// Cached display names, refreshed after resolving the message anchors.
  final String chatTitle;
  final String chapterTitle;

  /// Set for a section reference; the chapter is the section's parent.
  final String? sectionTitle;

  final String summary;
  final String? decision;
  String? get displayDecision => outlineDecision([?decision]);

  /// The chapter's dates ("Sep 2 – Sep 4", "since Sep 2"); empty for a section.
  final String when;
  final String colour;

  /// A chapter's sections, in order (empty for a section reference).
  final List<ChatRefSection> sections;

  bool get isSection => level == ChatRefLevel.section;

  /// What the reference is called: the section, or the chapter.
  String get chapterName => outlineNameGist(chapterTitle, summary).name;
  String get title => outlineNameGist(
    sectionTitle ?? chapterTitle,
    summary,
    chapterName: isSection ? chapterName : null,
  ).name;
  String get gist => outlineNameGist(
    sectionTitle ?? chapterTitle,
    summary,
    chapterName: isSection ? chapterName : null,
  ).gist;

  /// The breadcrumb on cards and in the picker: "Chat › Chapter".
  String get crumb => '$chatTitle › $chapterName';

  /// The inline label: the crumb, plus "› Section" only when the section is
  /// the target.
  String get label => isSection ? '$crumb › $title' : crumb;

  /// The block's text, which notes search indexes: the summary.
  String get searchText => summary;

  factory ChatReference.fromChapter({
    required String sessionId,
    required String chatTitle,
    required OutlineChapter chapter,
  }) => ChatReference(
    level: ChatRefLevel.chapter,
    sessionId: sessionId,
    firstMessageId: chapter.firstMessageId,
    lastMessageId: chapter.lastMessageId,
    chatTitle: chatTitle,
    chapterTitle: chapter.title,
    summary: chapter.summary,
    decision: chapter.decision,
    when: chapter.when,
    colour: chatRefBandFor(chapter.id),
    sections: [
      for (final s in chapter.sections)
        ChatRefSection(
          id: s.id,
          title: s.title,
          firstMessageId: s.firstMessageId,
          lastMessageId: s.lastMessageId,
          colour: chatRefBandFor(s.id),
          decision: s.decision,
        ),
    ],
  );

  factory ChatReference.fromSection({
    required String sessionId,
    required String chatTitle,
    required OutlineChapter chapter,
    required OutlineSection section,
  }) => ChatReference(
    level: ChatRefLevel.section,
    sessionId: sessionId,
    firstMessageId: section.firstMessageId,
    lastMessageId: section.lastMessageId,
    chatTitle: chatTitle,
    chapterTitle: chapter.title,
    sectionTitle: section.title,
    summary: section.summary,
    decision: section.decision,
    colour: chatRefBandFor(chapter.id),
  );

  Map<String, Object?> toJson() => {
    'v': 1,
    'level': level.name,
    'sessionId': sessionId,
    'firstMessageId': firstMessageId,
    'lastMessageId': lastMessageId,
    'chatTitle': chatTitle,
    'chapterTitle': chapterTitle,
    if (sectionTitle != null) 'sectionTitle': sectionTitle,
    'summary': summary,
    if (decision != null) 'decision': decision,
    if (when.isNotEmpty) 'when': when,
    'colour': colour,
    if (sections.isNotEmpty) 'sections': [for (final s in sections) s.toJson()],
  };

  /// The block `target` payload.
  String toTarget() => jsonEncode(toJson());

  /// Reads a block's target; null when it is missing or not a reference.
  static ChatReference? fromTarget(String? target) {
    if (target == null || target.isEmpty) return null;
    try {
      final j = jsonDecode(target) as Map<String, Object?>;
      return ChatReference(
        level: ChatRefLevel.values.byName(j['level'] as String),
        sessionId: j['sessionId'] as String,
        firstMessageId: j['firstMessageId'] as String,
        lastMessageId: j['lastMessageId'] as String,
        chatTitle: j['chatTitle'] as String,
        chapterTitle: j['chapterTitle'] as String,
        sectionTitle: j['sectionTitle'] as String?,
        summary: j['summary'] as String? ?? '',
        decision: j['decision'] as String?,
        when: j['when'] as String? ?? '',
        colour: j['colour'] as String? ?? 'sun',
        sections: [
          for (final s in (j['sections'] as List<Object?>? ?? const []))
            ChatRefSection.fromJson(s as Map<String, Object?>),
        ],
      );
    } catch (_) {
      // A damaged or newer payload shows as an unreadable block, not a crash.
      return null;
    }
  }
}

/// Opens the chat behind [reference] at its message range: the dock on
/// desktop, the full Agent screen on a phone. The store marks the range.
/// Cached cards stay readable with the agent off; opening needs the agent, and
/// a failure says why in a toast instead.
Future<void> openChatReference(
  BuildContext context,
  ChatReference reference,
) async {
  final app = context.app;
  final store = AgentStore.of(app);
  final problem = await _openProblem(store, reference.sessionId);
  if (!context.mounted) return;
  if (problem != null) {
    showErrorToast(context, problem);
    return;
  }
  // Only the visible presentation takes the span. The dock is hidden while
  // the full Agent route exists, so references opened there target the screen.
  final mobile = context.mobile;
  final screen =
      mobile ||
      PageStackScope.of(context).routes
          .any((route) => route.settings.name == agentRouteName);
  await store.openReference(
    reference.sessionId,
    reference.firstMessageId,
    reference.lastMessageId,
    presentation: screen ? AgentPresentation.screen : AgentPresentation.dock,
  );
  if (!context.mounted) return;
  if (screen) {
    unawaited(openAgent(context));
  } else {
    app.ui
      ..agentPanelOpen = true
      ..agentSessionId = store.openId
      ..changed();
  }
}

/// Why the chat cannot be opened now, or null when it can. The agent must
/// open (it is null when the agent is off or failed) and the chat must be in
/// its session list.
Future<String?> _openProblem(AgentStore store, String sessionId) async {
  try {
    final agent = await store.ensureOpen();
    if (agent == null || !store.enabled) {
      return 'Turn the agent on to open this chat';
    }
    await store.refreshSessions();
    if (!store.sessions.any((s) => s.id == sessionId)) {
      return 'This chat is not on this device any more';
    }
    return null;
  } catch (_) {
    return 'Could not open this chat';
  }
}
