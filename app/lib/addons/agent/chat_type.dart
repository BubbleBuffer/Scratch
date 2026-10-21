import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../registry/note_type_registry.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'agent_screen.dart';
import 'agent_store.dart';
import 'chat_reference.dart';
import 'chat_reference_resolution.dart';
import 'chat_reference_widgets.dart';
import 'navigation.dart';
import 'outline_names.dart';

/// Chat bodies remain in the agent store. A note presentation carries only
/// the session identity and, for an embed, the existing opaque range target.
const chatNoteType = NoteType(
  id: 'chat',
  label: 'Chat',
  chip: chatChip,
  card: chatCard,
  embed: chatEmbed,
  fullPage: _fullPage,
  usesEditor: false,
  bodySource: 'agent',
);

Widget chatChip(BuildContext context, NoteArgs args) {
  final ref = ChatReference.fromTarget(args.target ?? '');
  if (ref != null) {
    return ChatReferenceChip(
      reference: ref,
      onTap: args.onOpen ?? () => unawaited(openChatReference(context, ref)),
    );
  }
  return ReferenceChip(
    label: args.title.isEmpty ? 'Untitled chat' : args.title,
    colour: context.colors.accent,
    onTap: args.onOpen ?? () => unawaited(_openSession(context, args.pageId)),
  );
}

Widget chatCard(BuildContext context, NoteArgs args) {
  if (_wholeChatId(args) != null) return chatEmbed(context, args);
  return args.target == null
      ? Padding(
          padding: const EdgeInsets.all(12),
          child: chatChip(context, args),
        )
      : chatEmbed(context, args);
}

Widget chatEmbed(BuildContext context, NoteArgs args) {
  final ref = ChatReference.fromTarget(args.target ?? '');
  if (ref == null) {
    final sessionId = _wholeChatId(args);
    if (sessionId == null) {
      return _placeholder(context, 'Chat reference unavailable');
    }
    return ChatSessionCard(
      key: args.instanceId == null
          ? null
          : ValueKey('chatwhole-${args.instanceId}'),
      sessionId: sessionId,
      onOpen: args.onOpen ?? () => unawaited(_openSession(context, sessionId)),
    );
  }
  return ChatReferenceCard(
    key: args.instanceId == null
        ? null
        : ValueKey('chatref-${args.instanceId}'),
    reference: ref,
    onCacheChanged: (current) {
      args.onTargetChanged?.call(current.toTarget());
      args.onFallbackChanged?.call(current.searchText);
    },
    onOpenInChat: args.onOpen ?? () => openChatReference(context, ref),
  );
}

/// The kit names a chat's index row `chat:<sessionId>`; the chat type takes the
/// bare session ID from it.
const _chatRowPrefix = 'chat:';

/// The session a whole-chat embed names: a bare ID or a `chat:<id>` row in the
/// target, or the same in the page ID when there is no target. A range payload
/// is JSON and names a chapter or section, so it returns null and keeps its
/// reference card.
String? _wholeChatId(NoteArgs args) {
  final target = args.target;
  final raw = target == null || target.isEmpty ? args.pageId : target;
  if (raw.isEmpty || raw.startsWith('{')) return null;
  final id = raw.startsWith(_chatRowPrefix)
      ? raw.substring(_chatRowPrefix.length)
      : raw;
  return id.isEmpty ? null : id;
}

Widget _placeholder(BuildContext context, String text) => Container(
  padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
  decoration: BoxDecoration(
    color: context.colors.bgSecondary,
    borderRadius: Radii.ctl,
  ),
  child: Text(
    text,
    maxLines: 1,
    overflow: TextOverflow.ellipsis,
    style: ts(context.type.uiSmall, context.colors.textTertiary),
  ),
);

Color _bandColour(BuildContext context, String name) {
  final c = context.colors;
  return spacePatch(name, c.dark ? Brightness.dark : Brightness.light)?.deep ??
      c.accent;
}

/// A whole chat by its session ID: its title and chapters, drawn as a folding
/// card like the chapter card. It reads the chat through the same source as
/// the reference cards, keeps nothing in the note and writes no title back.
/// A chat that is gone, or cannot be reached before any load, shows the
/// unavailable placeholder.
class ChatSessionCard extends StatefulWidget {
  const ChatSessionCard({
    super.key,
    required this.sessionId,
    this.onOpen,
    this.source,
  });

  final String sessionId;

  /// Null hides the "Open in chat" action.
  final VoidCallback? onOpen;

  /// Null reads through the agent store of the mounted app.
  final ChatReferenceSource? source;

  @override
  State<ChatSessionCard> createState() => _ChatSessionCardState();
}

class _ChatSessionCardState extends State<ChatSessionCard> {
  ChatReferenceSource? _source;
  AgentStore? _store;
  Timer? _timer;
  bool _loading = false;
  bool _settled = false;
  int _generation = 0;

  /// The last chat read; kept while the agent is unreachable, cleared when
  /// the chat is missing.
  ChatReferenceSnapshot? _snapshot;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.maybeOf(context);
    final store = app == null ? null : AgentStore.of(app);
    if (!identical(store, _store)) {
      _store?.removeListener(_changed);
      _store = store;
      _store?.addListener(_changed);
    }
    _source =
        widget.source ??
        (store == null ? null : AgentChatReferenceSource.of(store));
    _changed();
  }

  @override
  void didUpdateWidget(ChatSessionCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.sessionId != widget.sessionId ||
        !identical(oldWidget.source, widget.source)) {
      _generation++;
      _snapshot = null;
      _settled = false;
      _source =
          widget.source ??
          (_store == null ? null : AgentChatReferenceSource.of(_store!));
      _changed();
    }
  }

  void _changed() {
    if (_source?.enabled ?? false) {
      _timer ??= Timer.periodic(const Duration(seconds: 1), (_) => _changed());
    } else {
      _timer?.cancel();
      _timer = null;
    }
    // Store notifications can arrive during a parent build.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) unawaited(_resolve());
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  Future<void> _resolve() async {
    final source = _source;
    final generation = _generation;
    if (source == null || !source.enabled) {
      _generation++;
      if (!_settled) setState(() => _settled = true);
      return;
    }
    if (_loading) return;
    _loading = true;
    ChatReferenceSnapshot? snapshot;
    try {
      snapshot = await source.load(widget.sessionId);
    } catch (_) {
      // An unreachable agent keeps the last chat read, or the placeholder.
      snapshot = _snapshot;
    } finally {
      _loading = false;
    }
    if (!mounted || generation != _generation || !source.enabled) return;
    if (snapshot != _snapshot || !_settled) {
      setState(() {
        _snapshot = snapshot;
        _settled = true;
      });
    }
  }

  @override
  void dispose() {
    _generation++;
    _timer?.cancel();
    _store?.removeListener(_changed);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final snapshot = _snapshot;
    if (snapshot != null) return _card(context, snapshot);
    if (!_settled) return const SizedBox.shrink();
    return _placeholder(context, 'Chat unavailable');
  }

  Widget _card(BuildContext context, ChatReferenceSnapshot s) {
    final c = context.colors;
    final ty = context.type;
    final chapters = s.outline.chapters;
    final count = chapters.length == 1
        ? '1 chapter'
        : '${chapters.length} chapters';
    return FoldableStackCard(
      faceKey: const ValueKey('chatwhole-fold'),
      edge: c.accent,
      label: 'Chat',
      title: s.chatTitle.isEmpty ? 'Untitled chat' : s.chatTitle,
      titleMaxLines: 1,
      stacked: true,
      peek: Text(
        count,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: ts(ty.uiSmall, c.textTertiary),
      ),
      body: chapters.isEmpty
          ? null
          : MarkedList(
              divided: true,
              rows: [
                for (final chapter in chapters)
                  MarkedRow(
                    label: outlineNameGist(chapter.title, chapter.summary).name,
                    colour: _bandColour(context, chatRefBandFor(chapter.id)),
                    small: true,
                    oneLine: true,
                    padding: const EdgeInsets.symmetric(vertical: 3),
                  ),
              ],
            ),
      footer: (isOpen) => !isOpen || widget.onOpen == null
          ? null
          : Row(
              children: [
                const Spacer(),
                FooterAction(
                  key: const ValueKey('chatwhole-open'),
                  label: 'Open in chat',
                  onTap: widget.onOpen,
                ),
              ],
            ),
    );
  }
}

Widget _fullPage(BuildContext context, NoteArgs args) => AgentScreen(
  key: ValueKey('chat-page-${args.pageId}'),
  initialSessionId: args.pageId,
);

Future<void> _openSession(BuildContext context, String id) async {
  final store = AgentStore.of(context.app);
  if (await store.ensureOpen() == null || !context.mounted) return;
  await store.openSession(id);
  if (context.mounted) unawaited(openAgent(context));
}
