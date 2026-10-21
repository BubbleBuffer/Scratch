import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/addon_registry.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'chat_reference.dart';
import 'navigation.dart';

class AgentChatSearchSource extends AddonSearchSource {
  const AgentChatSearchSource();
  @override
  String get id => 'agent';
  @override
  String get label => 'Chats';
  @override
  String get groupId => 'chats';
  @override
  Set<String> get noteTypes => const {'chat'};
  @override
  String get emptyTitle => 'Untitled chat';
  @override
  Ico get icon => Ico.comment;
  @override
  bool get indexed => true;
  @override
  Future<void> open(BuildContext context, AddonSearchResult result) =>
      openChatHit(context, result.toHit(id));
}

/// Where a chat hit opens: the session and the message range, read from the
/// hit's `target` JSON. Titles are never used to find the chat.
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

  /// The reference level the chat opens at: a section, else a chapter (a
  /// whole-chat hit opens the chat at its range like a chapter does).
  ChatRefLevel get refLevel =>
      level == 'section' ? ChatRefLevel.section : ChatRefLevel.chapter;

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

const _chatOpenFailed = 'Could not open this chat';

/// Opens the chat behind a chat [hit]. A row with a message range opens the
/// chat at that range, through the same path as a chat reference card. A
/// title-only row has no range and opens the chat as an ordinary session.
/// A malformed target, or a chat the agent cannot open, says why in a toast.
Future<void> openChatHit(BuildContext context, Hit hit) async {
  final target = ChatSearchTarget.parse(hit.target);
  if (target == null) {
    showErrorToast(context, _chatOpenFailed);
    return;
  }
  if (target.anchored) {
    await openChatReference(
      context,
      ChatReference(
        level: target.refLevel,
        sessionId: target.sessionId,
        firstMessageId: target.firstMessageId,
        lastMessageId: target.lastMessageId,
        chatTitle: hit.title,
        chapterTitle: target.chapterTitle ?? '',
        sectionTitle: target.sectionTitle,
        summary: hit.snippet,
      ),
    );
    return;
  }
  final app = context.app;
  final store = AgentStore.of(app);
  final problem = await _sessionProblem(store, target.sessionId);
  if (!context.mounted) return;
  if (problem != null) {
    showErrorToast(context, problem);
    return;
  }
  // The presentation an anchored open uses: the full Agent screen on a phone
  // or while its route is open, else the dock.
  final screen =
      context.mobile ||
      PageStackScope.of(context).routes
          .any((route) => route.settings.name == agentRouteName);
  await store.openSession(target.sessionId);
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

/// Why an ordinary session cannot be opened now, or null when it can. The
/// checks and messages match [openChatReference]'s.
Future<String?> _sessionProblem(AgentStore store, String sessionId) async {
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
    return _chatOpenFailed;
  }
}
