// The chat reference block: a chapter or section of a chat, drawn as the
// folded card. Its target holds the reference JSON (ids plus cached names);
// its text is the summary, which notes search indexes. It is in the `/` menu
// as "Chat": an empty block opens the Chat picker to choose what to show.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../chat_reference.dart';
import '../chat_reference_insert.dart';
import '../../../registry/block_registry.dart';
import '../chat_type.dart';
import '../../../registry/note_type_registry.dart';
import '../../../ui/ui.dart';

Widget _body(BlockContext b) => renderChatRef(b, noteTypeRegistry);

Widget renderChatRef(BlockContext b, NoteTypeRegistry types) {
  final target = b.node.target;
  if (target == null || target.isEmpty) return ChatReferenceInsert(block: b);
  final ref = ChatReference.fromTarget(target);
  // The constant renderer remains usable by the standalone block tests.
  // Registered renderers resolve against their own add-on context.
  final type = types.of('chat') ?? chatNoteType;
  return type.embed(
    b.context,
    NoteArgs(
      pageId: ref?.sessionId ?? '',
      pageType: 'chat',
      title: b.node.text,
      target: target,
      instanceId: b.node.id,
      onTargetChanged: (payload) {
        if (b.node.target != payload) b.host.setTarget(b.node, payload);
      },
      onFallbackChanged: (text) {
        if (b.node.text != text) b.host.setText(b.node, text);
      },
    ),
  );
}

const chatRefBlock = BlockType(
  kind: Kinds.chatRef,
  label: 'Chat',
  slashLabel: 'Chat',
  hint: 'A chapter or section of a chat.',
  icon: Ico.comment,
  aliases: 'chat reference chapter section',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  inMenu: true,
  insertOnly: true,
  // A default group would start a second "Basic blocks" run in the `/`
  // menu after the AI entry. Sits in the AI section with Ask AI.
  group: 'AI',
  body: _body,
);

void registerChatRefBlocks(BlockRegistry r) => r.register(chatRefBlock);
