import 'navigation.dart';
import '../../services/addon_services.dart';

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../state/ui_state.dart';
import '../../ui/ui.dart';
import 'agent_screen.dart';
import 'agent_store.dart';

/// One dock outside page routes. A presentation navigator supplies the shared
/// chat's popovers; actual page navigation uses the root page-stack navigator.
class AgentPanelShell extends StatefulWidget {
  const AgentPanelShell({super.key, required this.child});
  final Widget child;
  @override
  State<AgentPanelShell> createState() => _AgentPanelShellState();
}

class _AgentPanelShellState extends State<AgentPanelShell> {
  AgentStore? _store;
  AppState? _app;
  StreamSubscription<AgentUiAction>? _actions;
  final _presentation = GlobalKey<NavigatorState>();
  bool _restored = false;
  bool _hadSession = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.maybeOf(context);
    if (identical(app, _app)) return;
    _store?.removeListener(_rememberSession);
    _store?.removeHostSwitchConfirm(_confirmSwitch);
    unawaited(_actions?.cancel());
    _app = app;
    _store = app == null ? null : AgentStore.of(app);
    _store?.addListener(_rememberSession);
    _store?.addHostSwitchConfirm(_confirmSwitch);
    _actions = _store?.uiActions.listen(_action);
    _restored = false;
    _hadSession = false;
  }

  void _rememberSession() {
    final app = _app, store = _store;
    if (app == null || store == null) return;
    if (store.openId != null) _hadSession = true;
    if ((_restored || _hadSession) && app.ui.agentSessionId != store.openId) {
      app.ui
        ..agentSessionId = store.openId
        ..changed();
    }
    if (mounted) setState(() {});
  }

  Future<void> _restoreSession() async {
    if (_restored) return;
    _restored = true;
    final app = _app!, store = _store!;
    final remembered = app.ui.agentSessionId;
    await store.ensureOpen();
    if (!mounted || !identical(store, _store)) return;
    if (store.openId == null) {
      final id = store.sessions.any((s) => s.id == remembered)
          ? remembered
          : store.sessions.isEmpty
          ? null
          : store.sessions.first.id;
      if (id != null) await store.openSession(id);
    }
    _rememberSession();
  }

  Future<void> _action(AgentUiAction action) async {
    final store = _store, app = _app;
    if (!mounted ||
        store == null ||
        app == null ||
        action.sessionId != store.visibleSessionId) {
      return;
    }
    final stack = PageStackScope.of(context);
    final route = stack.currentRoute;
    final agentRoute = route?.settings.name == agentRouteName;
    // Covered, closed and unrelated sessions cannot move the editor.
    if (!agentRoute &&
        (!app.ui.agentPanelOpen ||
            context.mobile ||
            (route != null && !route.isFirst && stack.idOf(route) == null))) {
      return;
    }
    try {
      final page = await app.vault.loadPage(action.pageId);
      if (!mounted ||
          page.inTrash ||
          action.sessionId != store.visibleSessionId) {
        return;
      }
      final current = stack.currentRoute;
      if (current?.settings.name != agentRouteName &&
          (!app.ui.agentPanelOpen ||
              context.mobile ||
              (current != null &&
                  !current.isFirst &&
                  stack.idOf(current) == null))) {
        return;
      }
      final target = stack.navigator?.overlay?.context;
      if (target == null || !target.mounted) return;
      if (context.mobile) {
        _mobileNotice(
          target,
          displayTitle(page.title, firstLine: PageMeta.of(page).firstLine),
          () => unawaited(
            openAgentPage(
              target,
              action.pageId,
              page.title,
              blocks: action.blockIds,
              sessionId: action.sessionId,
            ),
          ),
        );
      } else {
        await openAgentPage(
          target,
          action.pageId,
          page.title,
          blocks: action.blockIds,
          sessionId: action.sessionId,
        );
      }
    } catch (_) {
      // A stale or removed page target does not disrupt the conversation.
    }
  }

  Future<bool> _confirmSwitch(AgentHost from) =>
      confirmHostSwitch(context, from);

  @override
  void dispose() {
    _store?.removeListener(_rememberSession);
    _store?.removeHostSwitchConfirm(_confirmSwitch);
    unawaited(_actions?.cancel());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final app = _app, store = _store;
    if (app == null || store == null) return widget.child;
    return ListenableBuilder(
      listenable: app.ui,
      builder: (context, _) {
        final stack = PageStackScope.of(context);
        final show =
            !context.mobile &&
            store.enabled &&
            app.ui.agentPanelOpen &&
            !stack.routes.any((r) => r.settings.name == agentRouteName);
        if (show && !_restored) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) unawaited(_restoreSession());
          });
        }
        return CallbackShortcuts(
          bindings: {
            platformActivator(
              const SingleActivator(LogicalKeyboardKey.keyJ, control: true),
            ): () =>
                unawaited(toggleAgentPanel(context)),
          },
          child: Navigator(
            key: _presentation,
            onGenerateRoute: (_) => PageRouteBuilder<void>(
              pageBuilder: (_, _, _) => ListenableBuilder(
                listenable: Listenable.merge([app.ui, stack, store]),
                builder: (context, _) {
                  final dock =
                      !context.mobile &&
                      store.enabled &&
                      app.ui.agentPanelOpen &&
                      !stack.routes.any(
                        (r) => r.settings.name == agentRouteName,
                      );
                  return Row(
                    children: [
                      Expanded(child: widget.child),
                      if (dock)
                        SizedBox(
                          width: app.ui.agentPanelWidth,
                          child: AgentPanel(store: store),
                        ),
                    ],
                  );
                },
              ),
            ),
          ),
        );
      },
    );
  }
}

class AgentPanelToggle extends StatelessWidget {
  const AgentPanelToggle({super.key});
  @override
  Widget build(BuildContext context) {
    final app = AppScope.maybeOf(context);
    if (app == null || context.mobile || !AgentStore.of(app).enabled) {
      return const SizedBox.shrink();
    }
    return ListenableBuilder(
      listenable: app.ui,
      builder: (context, _) => IconBtn(
        Ico.comment,
        key: const ValueKey('agent-panel-toggle'),
        semanticLabel: 'Toggle agent panel (${shortcutHint('Ctrl+J')})',
        tooltip: 'Toggle agent panel',
        command: 'agent.open',
        selected: app.ui.agentPanelOpen,
        onTap: () => unawaited(toggleAgentPanel(context)),
      ),
    );
  }
}

class AgentPanel extends StatelessWidget {
  const AgentPanel({super.key, required this.store});
  final AgentStore store;

  Future<void> _sessions(BuildContext context) async {
    final selected = await showPopover<String>(
      context,
      globalRectOf(context),
      (menu) => ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: (MediaQuery.sizeOf(menu).height - 32).clamp(120.0, 480.0),
        ),
        child: SingleChildScrollView(
          child: MenuList(
            width: 280,
            children: [
              for (final s in store.sessions)
                MenuItem(
                  key: ValueKey('agent-panel-session-${s.id}'),
                  label: s.title.trim().isEmpty
                      ? 'Untitled chat'
                      : store.displaySessionTitle(s.id, s.title),
                  selected: store.openId == s.id,
                  onTap: () => Navigator.of(menu).pop(s.id),
                ),
              if (store.sessions.isNotEmpty) const MenuSeparator(),
              MenuItem(
                key: const ValueKey('agent-panel-new-chat'),
                label: 'New chat',
                icon: Ico.plus,
                onTap: () => Navigator.of(menu).pop(''),
              ),
            ],
          ),
        ),
      ),
    );
    if (selected == null) return;
    if (selected.isEmpty) {
      await store.newChat();
    } else {
      await store.openSession(selected);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ui = context.app.ui;
    return DecoratedBox(
      key: const ValueKey('agent-panel'),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border(left: BorderSide(color: c.border)),
      ),
      child: Stack(
        children: [
          Padding(
            padding: const EdgeInsets.only(left: 6),
            child: Column(
              children: [
                SizedBox(
                  height: Layout.topbarHeight,
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    child: Row(
                      children: [
                        Expanded(
                          child: Builder(
                            builder: (anchor) => Hover(
                              key: const ValueKey('agent-panel-sessions'),
                              onTap: () => _sessions(anchor),
                              radius: Radii.md,
                              padding: const EdgeInsets.symmetric(
                                horizontal: 6,
                                vertical: 6,
                              ),
                              child: Row(
                                children: [
                                  Expanded(
                                    child: Text(
                                      store.displaySessionTitle(
                                        store.openId ?? '',
                                        store.state?.session.title ?? 'Agent',
                                      ),
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: ts(context.type.ui, c.textPrimary),
                                    ),
                                  ),
                                  Icon20(
                                    Ico.chevronDown,
                                    size: 14,
                                    color: c.iconSecondary,
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ),
                        IconBtn(
                          Ico.duplicate,
                          key: const ValueKey('agent-panel-full'),
                          semanticLabel: 'Open full agent',
                          onTap: () => unawaited(openAgent(context)),
                        ),
                        IconBtn(
                          Ico.close,
                          key: const ValueKey('agent-panel-close'),
                          semanticLabel: 'Close agent panel',
                          onTap: () {
                            ui
                              ..agentPanelOpen = false
                              ..changed();
                          },
                        ),
                      ],
                    ),
                  ),
                ),
                Expanded(child: AgentChat(store: store, panel: true)),
              ],
            ),
          ),
          Positioned(
            left: 0,
            top: 0,
            bottom: 0,
            width: 6,
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeLeftRight,
              child: GestureDetector(
                key: const ValueKey('agent-panel-resize'),
                behavior: HitTestBehavior.opaque,
                onHorizontalDragUpdate: (details) {
                  ui.agentPanelWidth = (ui.agentPanelWidth - details.delta.dx)
                      .clamp(
                        UiState.minAgentPanelWidth,
                        UiState.maxAgentPanelWidth,
                      )
                      .toDouble();
                  ui.changed();
                },
              ),
            ),
          ),
        ],
      ),
    );
  }
}

void _mobileNotice(BuildContext context, String title, VoidCallback open) {
  final overlay = Overlay.of(context);
  late final OverlayEntry entry;
  var removed = false;
  void remove() {
    if (removed) return;
    removed = true;
    entry.remove();
    entry.dispose();
  }

  entry = OverlayEntry(
    builder: (context) => _MobileAgentNotice(
      title: title,
      open: () {
        remove();
        open();
      },
      remove: remove,
    ),
  );
  overlay.insert(entry);
}

class _MobileAgentNotice extends StatefulWidget {
  const _MobileAgentNotice({
    required this.title,
    required this.open,
    required this.remove,
  });
  final String title;
  final VoidCallback open, remove;
  @override
  State<_MobileAgentNotice> createState() => _MobileAgentNoticeState();
}

class _MobileAgentNoticeState extends State<_MobileAgentNotice> {
  late final Timer _timer;
  @override
  void initState() {
    super.initState();
    _timer = Timer(const Duration(seconds: 5), widget.remove);
  }

  @override
  void dispose() {
    _timer.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Positioned(
    left: 16,
    right: 16,
    bottom: MediaQuery.viewInsetsOf(context).bottom + 24,
    child: MenuSurface(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 8, 8, 8),
        child: Row(
          children: [
            Expanded(
              child: Text(
                'Agent: ${displayTitle(widget.title)}',
                key: const ValueKey('agent-page-notice'),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: ts(context.type.ui, context.colors.textPrimary),
              ),
            ),
            TextBtn(
              'Open',
              key: const ValueKey('agent-page-notice-open'),
              onTap: widget.open,
            ),
          ],
        ),
      ),
    ),
  );
}
