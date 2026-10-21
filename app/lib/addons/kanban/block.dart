// The `/` menu entry for a kanban. Like the database entry, it only inserts:
// the editor stores the kit's embed kind and calls createTarget, which makes
// the kanban note under the page and points the embed at it.

import 'package:flutter/widgets.dart';

import '../../registry/block_codecs.dart';
import '../../registry/block_registry.dart';
import '../../registry/embed_block.dart';
import '../../ui/icon_x.dart';

/// The block kind name the slash item and the menu use.
const kanbanInsertKind = 'kanban';

BlockType kanbanInsertBlock({
  required Future<String?> Function(BuildContext context, String pageId)
  createTarget,
}) => BlockType(
  kind: kanbanInsertKind,
  label: 'Kanban',
  hint: 'Add a board of cards that link to notes.',
  iconX: Ix.columns,
  aliases: 'kanban board cards columns',
  group: 'Advanced',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  insertOnly: true,
  insertKind: embedKind,
  createTarget: createTarget,
  body: (_) => const SizedBox.shrink(),
  codecs: genericBlockCodecs,
);
