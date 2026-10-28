import '../../services/addon_services.dart';
// The Agent section (desktop): the chat list on the desk, the open chat on
// a sheet, and the composer under it.
//
// - Chats: newest first, a title filter, New chat, and rename or delete
//   from each row's menu (delete asks first).
// - AgentChat is shared with the editor dock: assistant markdown, compact
//   edits, work summaries with optional code, and the context-aware composer.
// - The composer: Enter sends, Shift+Enter is a new line. While a run is
//   going, Send steers it and Stop cancels it. The model is picked per
//   chat. Sending with no chat open starts one named after the message.
// - Phones: no chat list, so New chat is an icon in the top bar. Signed out
//   on Android it is off and the chat shows Sign in (see AgentChat).

export 'agent_cells.dart' show CodeCellRow;
export 'agent_chat.dart' show AgentChat, agentEmptyLine;

import 'package:flutter/widgets.dart';
import 'package:flutter/services.dart';

import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'agent_chat.dart';
import 'agent_store.dart';

class AgentScreen extends StatefulWidget {
  const AgentScreen({super.key, this.initialSessionId});
  final String? initialSessionId;

  @override
  State<AgentScreen> createState() => _AgentScreenState();
}

class _AgentScreenState extends State<AgentScreen> {
  late final AgentStore store = AgentStore.of(context.app);

  Future<bool> _confirmSwitch(AgentHost from) =>
      confirmHostSwitch(context, from);

  @override
  void dispose() {
    store.removeHostSwitchConfirm(_confirmSwitch);
    super.dispose();
  }

  @override
  void initState() {
    super.initState();
    store.addHostSwitchConfirm(_confirmSwitch);
    store.ensureOpen().then((_) {
      if (!mounted) return;
      final requested = widget.initialSessionId;
      if (requested != null) {
        store.openSession(requested);
      } else if (store.openId == null && store.sessions.isNotEmpty) {
        store.openSession(store.sessions.first.id);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return ColoredBox(
      key: const ValueKey('agent-screen'),
      color: c.desk,
      child: Row(
        children: [
          if (!context.mobile) const Sidebar(agentOpen: true),
          Expanded(
            child: Column(
              children: [
                SizedBox(
                  height: Layout.topbarHeight,
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 12),
                    child: Row(
                      children: [
                        if (!context.mobile) const SidebarReopenButton(),
                        if (!context.mobile) const CollapsedFeedbackButton(),
                        Icon20(Ico.comment, size: 16, color: c.iconSecondary),
                        const SizedBox(width: 6),
                        Text(
                          'Agent',
                          style: ts(
                            context.type.ui,
                            c.textPrimary,
                            weight: 400,
                          ),
                        ),
                        const Spacer(),
                        // Phones have no chat list: New chat lives here.
                        if (context.mobile && hubAgentPlatform)
                          ListenableBuilder(
                            listenable: store,
                            builder: (context, _) => IconBtn(
                              Ico.compose,
                              key: const ValueKey('agent-new-chat-mobile'),
                              size: 44,
                              glyph: 22,
                              semanticLabel: 'New chat',
                              onTap: store.enabled && !store.requiresHubSignIn
                                  ? () => store.newChat()
                                  : null,
                            ),
                          ),
                        const SyncIndicator(),
                      ],
                    ),
                  ),
                ),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                    child: ListenableBuilder(
                      listenable: store,
                      builder: (context, _) => Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          if (!context.mobile) ...[
                            SizedBox(
                              width: 248,
                              child: _SessionList(store: store),
                            ),
                            const SizedBox(width: 12),
                          ],
                          Expanded(
                            child: AgentChat(
                              store: store,
                              title: store.outline.chapters.isEmpty
                                  ? null
                                  : store.displaySessionTitle(
                                      store.openId ?? '',
                                      store.state?.session.title ?? 'Chat',
                                    ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------- chats

class _SessionList extends StatefulWidget {
  const _SessionList({required this.store});
  final AgentStore store;

  @override
  State<_SessionList> createState() => _SessionListState();
}

class _SessionListState extends State<_SessionList> {
  late final _filter = TextEditingController(text: widget.store.filter);
  final _filterFocus = FocusNode();
  final _renameCtl = TextEditingController();
  final _renameFocus = FocusNode();
  String? _renaming, _menuOpen;

  @override
  void dispose() {
    _filter.dispose();
    _filterFocus.dispose();
    _renameCtl.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  Future<void> _menu(BuildContext row, AgentSession s) async {
    setState(() => _menuOpen = s.id);
    final picked = await showPopover<String>(
      row,
      globalRectOf(row),
      (ctx) => MenuList(
        width: 200,
        children: [
          MenuItem(
            key: const ValueKey('agent-rename'),
            label: 'Rename',
            icon: Ico.compose,
            onTap: () => Navigator.of(ctx).pop('rename'),
          ),
          MenuItem(
            key: const ValueKey('agent-delete'),
            label: 'Delete',
            icon: Ico.trash,
            danger: true,
            onTap: () => Navigator.of(ctx).pop('delete'),
          ),
        ],
      ),
    );
    if (!mounted) return;
    setState(() => _menuOpen = null);
    if (picked == 'rename') {
      _renameCtl.text = s.title;
      _renameCtl.selection = TextSelection(
        baseOffset: 0,
        extentOffset: s.title.length,
      );
      setState(() => _renaming = s.id);
      _renameFocus.requestFocus();
    } else if (picked == 'delete') {
      final ok = await showConfirm(
        context,
        title: 'Delete this chat?',
        message:
            '"${s.title}" and its history are deleted from this device. Pages it edited keep their changes.',
        confirmLabel: 'Delete',
        danger: true,
      );
      if (ok) await widget.store.deleteSession(s.id);
    }
  }

  void _commitRename() {
    final id = _renaming;
    if (id == null) return;
    setState(() => _renaming = null);
    widget.store.renameSession(id, _renameCtl.text);
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final c = context.colors;
    final list = store.visibleSessions;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SearchField(
          fieldKey: const ValueKey('agent-filter'),
          controller: _filter,
          focusNode: _filterFocus,
          placeholder: 'Filter chats',
          onChanged: store.setFilter,
        ),
        const SizedBox(height: 8),
        SecondaryButton(
          key: const ValueKey('agent-new-chat'),
          label: 'New chat',
          height: 34,
          onTap: store.enabled ? () => store.newChat() : null,
        ),
        const SizedBox(height: 10),
        Expanded(
          child: list.isEmpty
              ? Padding(
                  padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
                  child: Text(
                    store.sessions.isEmpty
                        ? 'No chats yet.'
                        : 'No chat matches.',
                    key: const ValueKey('agent-no-chats'),
                    style: tsRaw(13, 18, 400, c.textTertiary),
                  ),
                )
              : ListView(
                  padding: EdgeInsets.zero,
                  children: [
                    for (final s in list)
                      _renaming == s.id
                          ? _RenameRow(
                              key: ValueKey('agent-session-${s.id}'),
                              controller: _renameCtl,
                              focusNode: _renameFocus,
                              onSubmit: _commitRename,
                              onCancel: () => setState(() => _renaming = null),
                            )
                          : _SessionRow(
                              key: ValueKey('agent-session-${s.id}'),
                              session: s,
                              selected: store.openId == s.id,
                              menuOpen: _menuOpen == s.id,
                              onTap: () => store.openSession(s.id),
                              onMenu: (row) => _menu(row, s),
                            ),
                  ],
                ),
        ),
      ],
    );
  }
}

class _SessionRow extends StatefulWidget {
  const _SessionRow({
    super.key,
    required this.session,
    required this.selected,
    required this.menuOpen,
    required this.onTap,
    required this.onMenu,
  });
  final AgentSession session;
  final bool selected, menuOpen;
  final VoidCallback onTap;
  final void Function(BuildContext row) onMenu;

  @override
  State<_SessionRow> createState() => _SessionRowState();
}

class _SessionRowState extends State<_SessionRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final title = widget.session.title.trim().isEmpty
        ? 'Untitled chat'
        : AgentStore.of(context.app)
              .displaySessionTitle(widget.session.id, widget.session.title);
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Hover(
        onTap: widget.onTap,
        selected: widget.selected || widget.menuOpen,
        selectedColor: c.bgPressed,
        radius: Radii.md,
        onHoverChanged: (v) => setState(() => _hover = v),
        padding: const EdgeInsets.only(left: 10, right: 4),
        child: SizedBox(
          height: 32,
          child: Row(
            children: [
              Expanded(
                child: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(
                    context.type.ui,
                    c.textPrimary,
                    weight: widget.selected ? 700 : 400,
                  ),
                ),
              ),
              if (_hover || widget.menuOpen)
                Builder(
                  builder: (row) => IconBtn(
                    Ico.more,
                    key: ValueKey('agent-session-menu-${widget.session.id}'),
                    size: 24,
                    glyph: 16,
                    semanticLabel: 'Chat actions',
                    onTap: () => widget.onMenu(row),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _RenameRow extends StatelessWidget {
  const _RenameRow({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.onSubmit,
    required this.onCancel,
  });
  final TextEditingController controller;
  final FocusNode focusNode;
  final VoidCallback onSubmit, onCancel;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: CallbackShortcuts(
        bindings: {const SingleActivator(LogicalKeyboardKey.escape): onCancel},
        child: FieldFrame(
          height: 32,
          child: PlainField(
            key: const ValueKey('agent-rename-field'),
            controller: controller,
            focusNode: focusNode,
            style: ts(context.type.ui, c.textPrimary, weight: 400),
            placeholder: 'Chat name',
            textInputAction: TextInputAction.done,
            onSubmitted: (_) => onSubmit(),
          ),
        ),
      ),
    );
  }
}

/// Asks before opening a chat on the other host stops the reply running on
/// [from]. True: switch (the reply stops); false: keep the current host.
Future<bool> confirmHostSwitch(BuildContext context, AgentHost from) =>
    showConfirm(
      context,
      title: 'A reply is still running on ${from.label}.',
      message: 'Switch anyway? It will stop.',
      confirmLabel: 'Switch',
      danger: true,
    );
