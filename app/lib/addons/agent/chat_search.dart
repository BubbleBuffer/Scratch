import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/addon_registry.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'chat_reference.dart';
import 'chat_search_target.dart';
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

const _chatOpenFailed = 'Could not open this chat';

/// The chat reference level a search target opens at: a section, else a
/// chapter (a whole-chat hit opens the chat at its range like a chapter does).
extension ChatSearchTargetNavigation on ChatSearchTarget {
  ChatRefLevel get refLevel => level == chatSearchLevelSection
      ? ChatRefLevel.section
      : ChatRefLevel.chapter;
}

/// Opens a chat reference card's range. [openChatReference] by default; a test
/// passes its own to see what an anchored hit asks for.
typedef ChatReferenceOpener =
    Future<void> Function(BuildContext context, ChatReference reference);

/// Opens the chat behind a chat [hit]. A row with a message range opens the
/// chat at that range, through the same path as a chat reference card. A
/// title-only row has no range and opens the chat as an ordinary session.
/// A malformed target, or a chat the agent cannot open, says why in a toast.
Future<void> openChatHit(
  BuildContext context,
  Hit hit, {
  ChatReferenceOpener openReference = openChatReference,
}) async {
  final target = ChatSearchTarget.parse(hit.target);
  if (target == null) {
    showErrorToast(context, _chatOpenFailed);
    return;
  }
  if (target.anchored) {
    await openReference(
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
