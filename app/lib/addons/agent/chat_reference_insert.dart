
import 'package:flutter/widgets.dart';

import '../../registry/block_registry.dart';
import '../../src/rust/api/agent.dart' show OutlineChapter;
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'chat_reference_picker.dart';

/// The empty chat_ref created by the slash menu owns its picker. This uses
/// the registry's existing block body contract, so the page editor needs no
/// special insertion case. Cancelling leaves a retryable empty reference.
class ChatReferenceInsert extends StatefulWidget {
  const ChatReferenceInsert({super.key, required this.block, this.load});
  final BlockContext block;
  final Future<List<ChatRefChat>> Function()? load;

  @override
  State<ChatReferenceInsert> createState() => _ChatReferenceInsertState();
}

class _ChatReferenceInsertState extends State<ChatReferenceInsert> {
  bool _choosing = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _pick();
    });
  }

  Future<List<ChatRefChat>> _load() async {
    final store = AgentStore.of(context.app);
    final agent = await store.ensureOpen();
    if (agent == null || !store.enabled) {
      throw StateError('Turn the agent on to choose a chat');
    }
    await store.refreshSessions();
    final chats = <ChatRefChat>[];
    for (final session in store.sessions) {
      // One chat whose outline will not load still lists, with no chapters.
      final chapters = await agent
          .sessionOutline(sessionId: session.id)
          .then((o) => o.chapters, onError: (_) => const <OutlineChapter>[]);
      chats.add(
        ChatRefChat(
          sessionId: session.id,
          title: session.title.trim().isEmpty
              ? 'Untitled chat'
              : store.displaySessionTitle(session.id, session.title),
          chapters: chapters,
        ),
      );
    }
    return chats;
  }

  Future<void> _pick() async {
    if (_choosing) return;
    setState(() => _choosing = true);
    final reference = await showChatReferencePicker(
      context,
      load: widget.load ?? _load,
    );
    if (!mounted) return;
    if (reference != null) {
      final block = widget.block;
      block.host.setTarget(block.node, reference.toTarget());
      block.host.setText(block.node, reference.searchText);
    }
    if (mounted) setState(() => _choosing = false);
  }

  @override
  Widget build(BuildContext context) => Hover(
    key: ValueKey('chatref-pick-${widget.block.node.id}'),
    onTap: _choosing ? null : _pick,
    radius: Radii.md,
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
    background: context.colors.bgSecondary,
    child: Row(
      children: [
        Icon20(Ico.comment, color: context.colors.iconSecondary),
        const SizedBox(width: 8),
        Expanded(
          child: Text(
            _choosing ? 'Choose a chat reference…' : 'Choose a chat reference',
            style: ts(context.type.ui, context.colors.textSecondary),
          ),
        ),
      ],
    ),
  );
}
