import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/page_link.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../state/editor_focus.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'agent_screen.dart';

/// The route name of the Agent screen.
const agentRouteName = '/agent';

/// Opens the Agent screen over Home, including Android's Hub sign-in state.
Future<void> openAgent(BuildContext context) async {
  final app = AppScope.maybeOf(context);
  if (app == null || !AgentStore.of(app).enabled) return;
  final nav =
      PageStackScope.maybeOf(context)?.navigator ?? Navigator.of(context);
  final sheets = PageStackScope.of(context);
  if (sheets.currentRoute?.settings.name == agentRouteName) return;
  nav.popUntil((r) => r.isFirst);
  await nav.push(
    pageRoute(
      (ctx) => context.mobile && hubAgentPlatform
          ? ColoredBox(
              color: ctx.colors.desk,
              child: Padding(
                padding: EdgeInsets.only(
                  bottom: MediaQuery.viewInsetsOf(ctx).bottom,
                ),
                child: const SafeArea(child: AgentScreen()),
              ),
            )
          : const AgentScreen(),
      motion: Motion.of(context),
      mobile: context.mobile,
      settings: const RouteSettings(name: agentRouteName),
    ),
  );
}

/// Opens the dock beside the current editor, preserving the selected chat.
Future<void> toggleAgentPanel(BuildContext context) async {
  final app = AppScope.maybeOf(context);
  if (app == null || !AgentStore.of(app).enabled) return;
  if (context.mobile) {
    unawaited(openAgent(context));
    return;
  }
  app.ui
    ..agentPanelOpen = !app.ui.agentPanelOpen
    ..changed();
}

/// Hands the visible selection to the composer without sending a message.
Future<void> askAgent(BuildContext context) async {
  final app = AppScope.maybeOf(context);
  if (app == null || !AgentStore.of(app).enabled) return;
  EditorFocus.forApp(app).requestComposer();
  if (context.mobile) {
    unawaited(openAgent(context));
  } else {
    app.ui
      ..agentPanelOpen = true
      ..changed();
  }
}

/// Agent links finish when navigation starts, not when the page later closes.
/// Full-screen chat hands off to the one shared dock on desktop.
Future<void> openAgentPage(
  BuildContext context,
  String id,
  String title, {
  List<String> blocks = const [],
  String? sessionId,
}) async {
  final app = context.app;
  final store = AgentStore.of(app);
  if (sessionId != null && store.openId != sessionId) {
    await store.openSession(sessionId);
    if (!context.mounted) return;
  }
  final stack = PageStackScope.of(context);
  // The root overlay survives removal of the full-screen chat route.
  final pageContext = stack.navigator?.overlay?.context ?? context;
  if (!context.mobile) {
    app.ui
      ..agentPanelOpen = true
      ..agentSessionId = store.openId
      ..changed();
    const AddonServices().closeRoute(context, agentRouteName);
  }
  stack.queueHighlights(id, blocks);
  unawaited(
    openPage(
      pageContext,
      id,
      title,
      initialBlock: blocks.isEmpty ? null : blocks.first,
    ),
  );
}

/// A `notes://page` link in agent chat opens the page in-app at its block.
/// A missing or trashed page gets a toast instead.
Future<void> openAgentPageLink(BuildContext context, PageLink link) async {
  String? title;
  try {
    final page = await context.app.vault.loadPage(link.pageId);
    if (!page.inTrash) title = page.title;
  } catch (_) {
    title = null;
  }
  if (!context.mounted) return;
  if (title == null) {
    showErrorToast(context, PageLink.missingMessage);
    return;
  }
  await openAgentPage(context, link.pageId, title, blocks: [?link.blockId]);
}
