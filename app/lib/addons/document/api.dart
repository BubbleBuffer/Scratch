import 'package:flutter/widgets.dart';

import '../../registry/addon_registry.dart';
import '../../registry/note_type_registry.dart';
import '../../ui/note_preview.dart';
import '../../ui/tokens.dart';

class DocumentAddon extends Addon {
  const DocumentAddon();
  @override
  String get id => 'document';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) => ctx.registerNoteType(documentNoteType);
}

Widget _editor(BuildContext context, NoteArgs args) => args.editor;

Widget _embed(BuildContext context, NoteArgs args) => Column(
  crossAxisAlignment: CrossAxisAlignment.stretch,
  children: [
    Padding(
      padding: const EdgeInsets.symmetric(horizontal: Space.sm, vertical: Space.sm),
      child: noteChip(context, args),
    ),
    args.editor,
  ],
);

final documentNoteType = NoteType(
  id: 'document',
  label: 'Document',
  chip: noteChip,
  card: noteCard,
  embed: _embed,
  fullPage: _editor,
);
