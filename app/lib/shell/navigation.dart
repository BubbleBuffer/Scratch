// Moving between screens: open a page, create one, send one to the trash.
// Desktop internal links retain the pages below as sheets. External entry
// points replace those page routes; Home always remains underneath.

import 'dart:convert';
import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/page_link.dart';
import '../editor/page_screen.dart';
import '../features/bakeoff.dart';
import '../state/app_state.dart';
import '../state/page_store.dart';
import '../state/tree_store.dart';
import '../ui/toast.dart';
import '../ui/ui.dart';
import 'page_stack.dart';
import 'mobile_page_history.dart';
import 'addons.dart';
export '../addons/agent/api.dart'
    show
        AgentPanelToggle,
        agentRouteName,
        openAgent,
        toggleAgentPanel,
        askAgent,
        openAgentPage;
export 'page_stack.dart'
    show
        PageStackScope,
        PageStackNavigator,
        PageLabel,
        pageLabel,
        pageLabelOf,
        firstLineStyle;

/// The navigator every command and shell action uses.
final appNavigatorKey = GlobalKey<NavigatorState>();

/// Opens page [id]. Returns when the page is closed and saved. The open
/// page is remembered, so the next start reopens it. [stack] pushes internal
/// desktop navigation; external callers use the replacement default.
/// Mobile links share one bounded history, including legacy database callers.
Future<void> openPage(
  BuildContext context,
  String id,
  String title, {
  bool isNew = false,
  String? initialBlock,
  bool stack = false,
  bool preserveMobileStack = false,
}) async {
  final nav =
      PageStackScope.maybeOf(context)?.navigator ?? Navigator.of(context);
  final mobile = context.mobile;
  final motion = Motion.of(context);
  final pages = context.read<PageStore>();
  final tree = context.read<TreeStore>();
  final ui = context.app.ui;
  final sheets = PageStackScope.of(context);
  sheets.titles[id] = title;
  if (mobile) {
    final entry = MobilePageEntry(
      id,
      title,
      isNew: isNew,
      initialBlock: initialBlock,
    );
    final host = MobilePageHistoryScope.maybeOf(context);
    if (host != null && host.mounted) {
      await host.append(entry);
      await host.widget.route.popped;
      await pages.lastSave;
      return;
    }
    // Home/search/agent entry starts a new history. Removing the old route
    // also releases its sole live editor; await its save before reopening.
    nav.popUntil((r) => r.isFirst);
    await pages.lastSave;
    if (!nav.mounted) return;
    ui
      ..lastPage = id
      ..changed();
    tree.visit(id);
    Bakeoff.instance.pageOpenStart(title);
    final route = MobilePageHistoryRoute(entry, motion);
    await nav.push(route);
    // A later Home entry may have replaced this history already.
    if (ui.lastPage == route.history.current.id) {
      ui
        ..lastPage = null
        ..changed();
    }
    await pages.lastSave;
    return;
  }
  tree.visit(id);
  if (!mobile) {
    // Pages open over Home, not over the Agent screen.
    if (!stack) _closePanels(nav, sheets);
    final existing = sheets.routeFor(id);
    if (existing != null) {
      sheets.popTo(existing);
      if (initialBlock != null) sheets.revealBlocks[id]?.call(initialBlock);
      ui
        ..lastPage = id
        ..changed();
      await existing.popped;
      await pages.lastSave;
      return;
    }
    if (!stack) sheets.clearImmediately();
    if (sheets.pages.length >= 4) nav.removeRoute(sheets.pages.first);
  }
  ui
    ..lastPage = id
    ..changed();
  Bakeoff.instance.pageOpenStart(title);
  await nav.push(
    pageRoute(
      (_) => PageScreen(
        pageId: id,
        initialTitle: title,
        isNew: isNew,
        initialBlock: initialBlock,
      ),
      motion: motion,
      mobile: mobile,
      desktopSheet: !mobile,
      settings: RouteSettings(name: '/page', arguments: id),
    ),
  );
  // Closed: back home, unless another page replaced this one.
  if (!mobile || ui.lastPage == id) {
    ui
      ..lastPage = sheets.topId
      ..changed();
  }
  await pages.lastSave;
}

/// Creates an empty page (inside [parent], or at the top level) and opens it.
Future<void> createPage(BuildContext context, {String? parent}) async {
  final vault = context.app.vault;
  final tree = context.read<TreeStore>();
  final id = await vault.createPage(parent: parent);
  if (parent != null) tree.expand(parent);
  if (!context.mounted) return;
  await openPage(context, id, '', isNew: true);
}

/// Moves page [id] and its subpages to the trash. An open editor of it
/// closes itself.
Future<void> trashPage(BuildContext context, String id) =>
    context.read<PageStore>().trash(id);

/// Back to home.
void goHome(BuildContext context) =>
    (PageStackScope.maybeOf(context)?.navigator ?? Navigator.of(context))
        .popUntil((r) => r.isFirst);

/// Notification payloads use the same page/block navigation as Today.
Future<void> openReminder(BuildContext context, String payload) async {
  if (payload == 'today') {
    context.app.agenda.backToToday();
    goHome(context);
    return;
  }
  try {
    final data = jsonDecode(payload) as Map<String, dynamic>;
    final id = data['page'] as String;
    final block = data['block'] as String?;
    final page = await context.app.vault.loadPage(id);
    if (!context.mounted || page.inTrash) return;
    await openPage(context, id, page.title, initialBlock: block);
  } catch (_) {
    // A removed target or obsolete payload should not prevent app startup.
  }
}

/// Opens a `notes://page` link (cold start, a forwarded Windows link, Android
/// VIEW, or a page mark) at its block, replacing the page shown as an external
/// open does. A missing or trashed page gets a toast and nothing else changes.
Future<void> openPageLink(BuildContext context, PageLink link) async {
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
  await openPage(context, link.pageId, title, initialBlock: link.blockId);
}

void _closePanels(NavigatorState nav, PageStackNavigator sheets) {
  final routes = {
    for (final panel in appAddonContext.shellPanels.all) panel.routeName,
  };
  for (final route
      in sheets.routes
          .where((r) => routes.contains(r.settings.name))
          .toList()) {
    nav.removeRoute(route);
  }
}
