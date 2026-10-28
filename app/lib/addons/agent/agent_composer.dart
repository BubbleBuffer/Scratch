import '../../services/addon_services.dart';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../state/editor_focus.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';

/// Looks up the pages an `@` query offers; [mentionPages] unless a test
/// supplies its own.
typedef AgentMentionSource =
    Future<List<MentionItem>> Function(
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
  late final _focus = FocusNode(onKeyEvent: _onKey);
  EditorFocus? _editor;
  String? _pageId, _removedPage, _removedSelection;
  AgentContextPage? _autoPage;
  AgentContextSelection? _selection;
  final _chats = <String, AgentContextChat>{};
  final _mentions = <String, AgentContextPage>{};
  List<MentionItem>? _menu;
  int _active = 0, _mentionStart = 0, _request = 0, _queryEpoch = 0;
  bool _sending = false;

  @override
  void initState() {
    super.initState();
    _text.addListener(_textChanged);
  }

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
    _text.removeListener(_textChanged);
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

  void _textChanged() {
    if (!mounted) return;
    setState(() {});
    _findMentions();
  }

  Future<void> _findMentions() async {
    final epoch = ++_queryEpoch;
    final cursor = _text.selection.extentOffset;
    if (cursor < 0 || !_text.selection.isCollapsed) {
      setState(() => _menu = null);
      return;
    }
    final before = _text.text.substring(0, cursor);
    final match = RegExp(r'(?:^|\s)@([^\s@]*)$').firstMatch(before);
    if (match == null) {
      setState(() => _menu = null);
      return;
    }
    final pages = await widget.mentionSource(context, match[1]!, '');
    if (!mounted || epoch != _queryEpoch) return;
    setState(() {
      _mentionStart = before.lastIndexOf('@');
      final query = match[1]!.toLowerCase();
      _menu = [
        ...pages,
        ...widget.store.sessions
            .where(
              (chat) =>
                  chat.id != widget.store.openId &&
                  chat.title.toLowerCase().contains(query),
            )
            .take(4)
            .map(
              (chat) => MentionItem.chat(
                chat.id,
                widget.store.displaySessionTitle(chat.id, chat.title),
              ),
            ),
      ];
      _active = 0;
    });
  }

  void _pick(MentionItem item) {
    final end = _text.selection.extentOffset;
    final inserted = '@${item.label} ';
    setState(() {
      if (item.isChat) {
        _chats[item.id] = AgentContextChat(id: item.id, title: item.label);
      } else {
        _mentions[item.id] = AgentContextPage(id: item.id, title: item.label);
      }
      _menu = null;
    });
    _text.value = TextEditingValue(
      text: _text.text.replaceRange(_mentionStart, end, inserted),
      selection: TextSelection.collapsed(
        offset: _mentionStart + inserted.length,
      ),
    );
    _focus.requestFocus();
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (_menu != null && event is KeyDownEvent) {
      if (event.logicalKey == LogicalKeyboardKey.escape) {
        setState(() => _menu = null);
        return KeyEventResult.handled;
      }
      if (_menu!.isNotEmpty &&
          (event.logicalKey == LogicalKeyboardKey.arrowDown ||
              event.logicalKey == LogicalKeyboardKey.arrowUp)) {
        setState(
          () => _active =
              (_active +
                  (event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1)) %
              _menu!.length,
        );
        return KeyEventResult.handled;
      }
      if (_menu!.isNotEmpty && event.logicalKey == LogicalKeyboardKey.enter) {
        _pick(_menu![_active]);
        return KeyEventResult.handled;
      }
    }
    final enter =
        event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter;
    if (!enter || HardwareKeyboard.instance.isShiftPressed) {
      return KeyEventResult.ignored;
    }
    if (event is KeyDownEvent) _send();
    return KeyEventResult.handled;
  }

  Future<void> _send() async {
    final text = _text.text.trim();
    if (text.isEmpty || _sending) return;
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
    setState(() => _sending = true);
    try {
      if (store.openId == null) {
        final first = text.split('\n').first.trim();
        final session = await store.createSession(
          title: first.length > 60 ? '${first.substring(0, 60)}…' : first,
        );
        if (session == null) return;
      }
      await store.send(text, context: context);
      if (mounted && store.error == null) {
        _text.clear();
        setState(() {
          _mentions.clear();
          _chats.clear();
          _selection = null;
          _menu = null;
          _removedPage = null;
          _removedSelection = null;
          _captureEditor();
        });
      }
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  Widget _chip(
    String label,
    Key key,
    VoidCallback remove, {
    Ico icon = Ico.page,
  }) {
    final c = context.colors;
    return Container(
      key: key,
      padding: const EdgeInsets.only(left: 6),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.pill),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon20(icon, size: 12, color: c.iconTertiary),
          const SizedBox(width: 4),
          ConstrainedBox(
            constraints: BoxConstraints(maxWidth: widget.panel ? 190 : 280),
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textSecondary),
            ),
          ),
          IconBtn(
            Ico.close,
            key: ValueKey('${(key as ValueKey).value}-remove'),
            size: 24,
            glyph: 12,
            semanticLabel: 'Remove context',
            onTap: remove,
          ),
        ],
      ),
    );
  }

  String get _selectionLabel {
    final s = _selection!;
    if (s.blockIds.length > 1) return 'Selection · ${s.blockIds.length} blocks';
    final words = s.text.replaceAll(RegExp(r'\s+'), ' ').trim();
    if (words.isEmpty) return 'Selection · 1 block';
    return 'Selection · “${words.length > 35 ? '${words.substring(0, 35)}…' : words}”';
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final c = context.colors;
    final pages = context.watch<PagesStore>();
    final running = store.running;
    final model = store.modelOf(store.openId);
    final models = store.shownModels(current: model);
    final showModel = models.isNotEmpty && model != null;
    final showHost = store.canPickHost;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (_menu != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 5),
            child: MentionMenu(
              key: const ValueKey('agent-mentions'),
              items: _menu!,
              active: _active,
              maxHeight: 180,
              onPick: _pick,
              onHover: (i) => setState(() => _active = i),
            ),
          ),
        GestureDetector(
          onTap: _focus.requestFocus,
          child: FieldFrame(
            padding: const EdgeInsets.fromLTRB(12, 10, 8, 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (_autoPage != null ||
                    _selection != null ||
                    _mentions.isNotEmpty ||
                    _chats.isNotEmpty) ...[
                  Wrap(
                    spacing: 4,
                    runSpacing: 4,
                    children: [
                      if (_autoPage != null)
                        _chip(
                          displayTitle(
                            _autoPage!.title,
                            firstLine:
                                pages.meta(_autoPage!.id)?.firstLine ?? '',
                          ),
                          const ValueKey('agent-context-page'),
                          () => setState(() {
                            _removedPage = _autoPage!.id;
                            _autoPage = null;
                          }),
                        ),
                      if (_selection != null)
                        _chip(
                          _selectionLabel,
                          const ValueKey('agent-context-selection'),
                          () => setState(() {
                            _removedSelection = _selectionKey(_editor!);
                            _selection = null;
                          }),
                          icon: Ico.text,
                        ),
                      for (final chat in _chats.values)
                        _chip(
                          chat.title,
                          ValueKey('agent-context-chat-${chat.id}'),
                          () => setState(() => _chats.remove(chat.id)),
                          icon: Ico.comment,
                        ),
                      for (final page in _mentions.values)
                        _chip(
                          displayTitle(
                            page.title,
                            firstLine: pages.meta(page.id)?.firstLine ?? '',
                          ),
                          ValueKey('agent-context-mention-${page.id}'),
                          () => setState(() => _mentions.remove(page.id)),
                        ),
                    ],
                  ),
                  const SizedBox(height: 7),
                ],
                Stack(
                  children: [
                    if (_text.text.isEmpty)
                      Positioned.fill(
                        child: IgnorePointer(
                          child: Text(
                            running ? 'Steer the agent…' : 'Ask the agent…',
                            style: ts(context.type.body, c.textTertiary),
                          ),
                        ),
                      ),
                    EditableText(
                      key: const ValueKey('agent-composer'),
                      controller: _text,
                      focusNode: _focus,
                      style: ts(context.type.body, c.textPrimary),
                      cursorColor: c.textPrimary,
                      backgroundCursorColor: c.textTertiary,
                      selectionColor: c.textSelection,
                      cursorWidth: 1.5,
                      minLines: 1,
                      maxLines: widget.panel ? 5 : 8,
                      keyboardType: TextInputType.multiline,
                      textInputAction: TextInputAction.newline,
                      selectionControls: noteSelectionControls,
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    if (showModel || showHost)
                      Expanded(
                        child: Align(
                          alignment: Alignment.centerLeft,
                          child: FittedBox(
                            fit: BoxFit.scaleDown,
                            alignment: Alignment.centerLeft,
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                // Where the chat runs; fixed once it has
                                // messages, so it shows without a menu.
                                if (showHost)
                                  SelectButton<AgentHost>(
                                    key: const ValueKey('agent-runner'),
                                    title: 'Runs on',
                                    value: store.host,
                                    options: store.usableHosts,
                                    labelOf: (h) => h.label,
                                    onChanged: store.hostLocked || running
                                        ? null
                                        : (h) => store.pickHost(h),
                                  ),
                                if (showModel)
                                  SelectButton<String>(
                                    key: const ValueKey('agent-model'),
                                    title: 'Model',
                                    value: model,
                                    options: models,
                                    labelOf: store.modelLabel,
                                    onChanged: (m) =>
                                        store.pickModel(store.openId, m),
                                  ),
                              ],
                            ),
                          ),
                        ),
                      )
                    else
                      const Spacer(),
                    if (running) ...[
                      SecondaryButton(
                        key: const ValueKey('agent-stop'),
                        label: 'Stop',
                        height: 32,
                        onTap: store.stop,
                      ),
                      const SizedBox(width: 8),
                    ],
                    PrimaryButton(
                      key: const ValueKey('agent-send'),
                      label: running ? 'Steer' : 'Send',
                      height: 32,
                      icon: Ico.arrowUp,
                      onTap: _send,
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
