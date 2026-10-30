import '../../services/addon_services.dart';

import 'package:agent_kit/agent_kit.dart' as kit;
import 'package:agent_kit_core/agent_kit_core.dart'
    as kit
    show KitChoice, KitRef;
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../state/editor_focus.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';

/// Looks up the pages an `@` query offers; [mentionPages] unless a test
/// supplies its own.
typedef AgentMentionSource = Future<List<MentionItem>> Function(
  BuildContext context,
  String query,
  String exclude,
);

class AgentComposer extends StatefulWidget {
  const AgentComposer({
    super.key,
    required this.store,
    this.panel = false,
    this.mentionSource = mentionPages,
    this.controller,
  });
  final AgentStore store;
  final bool panel;
  final AgentMentionSource mentionSource;

  /// The draft, when the host wants to read it; the composer makes its own
  /// otherwise.
  final TextEditingController? controller;
  @override
  State<AgentComposer> createState() => _AgentComposerState();
}

class _AgentComposerState extends State<AgentComposer> {
  final _ownText = TextEditingController();
  TextEditingController get _text => widget.controller ?? _ownText;
  final _focus = FocusNode();
  EditorFocus? _editor;
  String? _pageId, _removedPage, _removedSelection;
  AgentContextPage? _autoPage;
  AgentContextSelection? _selection;
  final _chats = <String, AgentContextChat>{};
  final _mentions = <String, AgentContextPage>{};
  final _found = <String, MentionItem>{};
  int _request = 0;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final editor = EditorFocus.forApp(context.app);
    if (editor == _editor) return;
    _editor?.removeListener(_editorChanged);
    _editor = editor..addListener(_editorChanged);
    _captureEditor();
    final requestedSelection = editor.takeComposerSelection();
    if (!widget.panel && requestedSelection != null) {
      _autoPage = AgentContextPage(
        id: requestedSelection.pageId,
        title: requestedSelection.pageTitle,
      );
      if (requestedSelection.text.isNotEmpty ||
          requestedSelection.blockIds.isNotEmpty) {
        _selection = requestedSelection;
      }
    }
    _request = editor.composerRequest;
    if ((widget.panel && _request > 0) || requestedSelection != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _focus.requestFocus();
      });
    }
  }

  @override
  void dispose() {
    _editor?.removeListener(_editorChanged);
    _ownText.dispose();
    _focus.dispose();
    super.dispose();
  }

  String _selectionKey(EditorFocus e) =>
      '${e.pageId}:${e.blockIds.join(',')}:${e.text}';
  void _captureEditor({bool requested = false}) {
    if (!widget.panel) return;
    final e = _editor!;
    if (e.pageId == null) {
      _pageId = null;
      _autoPage = null;
      _selection = null;
      return;
    }
    if (e.pageId != _pageId && e.pageId != null) {
      _pageId = e.pageId;
      _removedPage = null;
      _removedSelection = null;
      _selection = null;
    }
    if (requested) {
      _removedPage = null;
      _removedSelection = null;
    }
    if (e.pageId != null && _removedPage != e.pageId) {
      _autoPage = AgentContextPage(id: e.pageId!, title: e.pageTitle);
    }
    // Retain the selected passage when the user moves focus to the composer.
    // A newly selected passage replaces it; changing pages clears it.
    if (e.pageId != null &&
        (e.text.isNotEmpty || e.blockIds.isNotEmpty) &&
        _removedSelection != _selectionKey(e)) {
      _selection = AgentContextSelection(
        pageId: e.pageId!,
        pageTitle: e.pageTitle,
        blockIds: [...e.blockIds],
        text: e.text,
      );
    }
  }

  void _editorChanged() {
    if (!mounted) return;
    final requested = _request != _editor!.composerRequest;
    if (!widget.panel && !requested) return;
    setState(() {
      _request = _editor!.composerRequest;
      _captureEditor(requested: requested);
      if (requested) {
        final selection = _editor!.takeComposerSelection();
        if (!widget.panel && selection != null) {
          _autoPage = AgentContextPage(
            id: selection.pageId,
            title: selection.pageTitle,
          );
          _selection =
              selection.text.isNotEmpty || selection.blockIds.isNotEmpty
              ? selection
              : null;
        }
      }
    });
    if (requested) _focus.requestFocus();
  }

  /// What `@` offers: the vault's pages, then other chats whose title matches.
  Future<List<kit.KitRef>> _suggest(String query) async {
    final pages = await widget.mentionSource(context, query, '');
    if (!mounted) return const [];
    final q = query.toLowerCase();
    final store = widget.store;
    final items = [
      ...pages,
      ...store.sessions
          .where(
            (chat) =>
                chat.id != store.openId && chat.title.toLowerCase().contains(q),
          )
          .take(4)
          .map(
            (chat) => MentionItem.chat(
              chat.id,
              store.displaySessionTitle(chat.id, chat.title),
            ),
          ),
    ];
    _found.clear();
    return [
      for (final it in items)
        if (!it.isDate)
          () {
            final ref = kit.KitRef(
              kind: it.isChat ? 'chat' : 'page',
              id: it.id,
              label: it.label,
            );
            _found['${ref.kind}-${ref.id}'] = it;
            return ref;
          }(),
    ];
  }

  kit.MentionRowLook _look(BuildContext context, kit.KitRef ref) {
    final it = _found['${ref.kind}-${ref.id}'];
    if (it == null || it.isChat) return const kit.MentionRowLook();
    final m = context.mobile;
    // An untitled page shows its first line, live while it is open.
    final stored = context.watch<PagesStore>().meta(it.id)?.firstLine ?? '';
    final live = PageStackScope.maybeOf(context)?.firstLines[it.id];
    final line = live != null && live.trim().isNotEmpty ? live : stored;
    return kit.MentionRowLook(
      leading: PageIcon(icon: it.icon, size: m ? 22 : 20),
      label: displayTitle(it.label, firstLine: line),
      firstLine: it.label.trim().isEmpty && line.trim().isNotEmpty,
    );
  }

  void _mentioned(kit.KitRef ref) {
    setState(() {
      if (ref.kind == 'chat') {
        _chats[ref.id] = AgentContextChat(id: ref.id, title: ref.label);
      } else {
        _mentions[ref.id] = AgentContextPage(id: ref.id, title: ref.label);
      }
    });
  }

  Future<bool> _send(kit.ComposerSubmission submission) async {
    final text = submission.text;
    final store = widget.store;
    final pages = Map<String, AgentContextPage>.of(_mentions);
    final autoPage = _autoPage;
    if (autoPage != null) {
      pages[autoPage.id] = autoPage;
    }
    final context = pages.isEmpty && _selection == null && _chats.isEmpty
        ? null
        : AgentMessageContext(
            pages: pages.values.toList(),
            chats: _chats.values.toList(),
            selection: _selection,
          );
    if (store.openId == null) {
      final first = text.split('\n').first.trim();
      final session = await store.createSession(
        title: first.length > 60 ? '${first.substring(0, 60)}…' : first,
      );
      if (session == null) return false;
    }
    await store.send(text, context: context);
    if (!mounted || store.error != null) return false;
    setState(() {
      _mentions.clear();
      _chats.clear();
      _selection = null;
      _removedPage = null;
      _removedSelection = null;
      _captureEditor();
    });
    return true;
  }

  String get _selectionLabel {
    final s = _selection!;
    if (s.blockIds.length > 1) return 'Selection · ${s.blockIds.length} blocks';
    final words = s.text.replaceAll(RegExp(r'\s+'), ' ').trim();
    if (words.isEmpty) return 'Selection · 1 block';
    return 'Selection · “${words.length > 35 ? '${words.substring(0, 35)}…' : words}”';
  }

  Key _chipKey(kit.KitRef ref) => switch (ref.kind) {
    'page' => const ValueKey('agent-context-page'),
    'selection' => const ValueKey('agent-context-selection'),
    'chat' => ValueKey('agent-context-chat-${ref.id}'),
    _ => ValueKey('agent-context-mention-${ref.id}'),
  };

  void _removeRef(kit.KitRef ref) => setState(() {
    switch (ref.kind) {
      case 'page':
        _removedPage = _autoPage?.id;
        _autoPage = null;
      case 'selection':
        _removedSelection = _selectionKey(_editor!);
        _selection = null;
      case 'chat':
        _chats.remove(ref.id);
      default:
        _mentions.remove(ref.id);
    }
  });

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final pages = context.watch<PagesStore>();
    final running = store.running;
    final model = store.modelOf(store.openId);
    final models = store.shownModels(current: model);
    final showModel = models.isNotEmpty && model != null;
    String titleOf(AgentContextPage p) =>
        displayTitle(p.title, firstLine: pages.meta(p.id)?.firstLine ?? '');
    return kit.AgentComposer(
      onSubmit: _send,
      running: running,
      onStop: store.stop,
      controller: _text,
      focusNode: _focus,
      placeholder: running ? 'Steer the agent…' : 'Ask the agent…',
      maxLines: widget.panel ? 5 : 8,
      chipMaxWidth: widget.panel ? 190 : 280,
      models: showModel
          ? [
              for (final m in models)
                kit.KitChoice(id: m, label: store.modelLabel(m)),
            ]
          : const [],
      modelId: model,
      showModelPick: showModel,
      onModelChanged: (c) => store.pickModel(store.openId, c.id),
      // Where the chat runs; fixed once it has messages, so it shows without
      // a menu.
      workspaces: store.canPickHost
          ? [
              for (final h in store.usableHosts)
                kit.KitChoice(id: h.name, label: h.label),
            ]
          : const [],
      workspaceId: store.host.name,
      workspaceTitle: 'Runs on',
      workspaceLocked: store.hostLocked || running,
      onWorkspaceChanged: (c) =>
          store.pickHost(AgentHost.values.firstWhere((h) => h.name == c.id)),
      showModeToggle: false,
      runningSendLabel: 'Steer',
      refs: [
        if (_autoPage != null)
          kit.KitRef(
            kind: 'page',
            id: _autoPage!.id,
            label: titleOf(_autoPage!),
          ),
        if (_selection != null)
          kit.KitRef(
            kind: 'selection',
            id: 'selection',
            label: _selectionLabel,
          ),
        for (final chat in _chats.values)
          kit.KitRef(kind: 'chat', id: chat.id, label: chat.title),
        for (final page in _mentions.values)
          kit.KitRef(kind: 'mention', id: page.id, label: titleOf(page)),
      ],
      chipKeyOf: _chipKey,
      onRemoveRef: _removeRef,
      mentions: _Mentions(_suggest),
      mentionLook: _look,
      onMentionPicked: _mentioned,
      maxMentions: 10,
      keys: const kit.ComposerKeys(
        field: ValueKey('agent-composer'),
        send: ValueKey('agent-send'),
        stop: ValueKey('agent-stop'),
        mentions: ValueKey('agent-mentions'),
        model: ValueKey('agent-model'),
        workspace: ValueKey('agent-runner'),
      ),
    );
  }
}

class _Mentions implements kit.MentionSource {
  const _Mentions(this._suggest);
  final Future<List<kit.KitRef>> Function(String) _suggest;

  @override
  String get trigger => '@';

  @override
  Future<List<kit.KitRef>> suggest(String query) => _suggest(query);
}
