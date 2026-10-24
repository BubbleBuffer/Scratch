// The kanban add-on's public API. The app and tests import only this file.
// A kanban is a note type of its own: it stores its columns and card ids in
// the page's type data, and every card is an ordinary note.

import 'package:flutter/widgets.dart';

import '../../core/vault.dart';
import '../../registry/addon_registry.dart';
import '../../registry/command_registry.dart';
import '../../registry/note_type_registry.dart';
import '../../services/legacy_host.dart';
import '../../ui/note_preview.dart';
import 'block.dart';
import 'create.dart';
import 'data.dart';
import 'view.dart';

export 'create.dart' show createKanban;
export 'data.dart' show KanbanColumn, KanbanData;
export 'source.dart' show KanbanBoardSource;
export 'runner_backend.dart'
    show
        AssignState,
        Runner,
        RunnerAssignment,
        RunnerAssignments,
        RunnerBackend,
        RunnerKind,
        RunnerScope,
        RunnerUnavailable;
export 'runner_chip.dart' show RunnerChip, runnerChipText;
export 'runner_fake.dart' show FakeRunnerBackend, demoRunnerAssignments;
export 'runner_picker.dart' show showRunnerPicker;
export 'runner_real.dart'
    show RealRunnerBackend, aiRunnerAssignments, runnerCardPrompt;
export 'view.dart' show KanbanBoard, KanbanPage;

class KanbanAddon extends Addon {
  const KanbanAddon();
  @override
  String get id => 'kanban';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    final noteTypes = ctx.noteTypes;
    ctx.registerNoteType(
      NoteType(
        id: kanbanPageType,
        label: 'Kanban',
        chip: noteChip,
        card: noteCard,
        usesEditor: false,
        embed: (context, args) => KanbanBoard(
          key: ValueKey('inline-kanban-${args.pageId}'),
          pageId: args.pageId,
          inline: true,
          noteTypes: noteTypes,
        ),
        fullPage: (context, args) => KanbanPage(
          key: ValueKey('kanban-page-${args.pageId}'),
          pageId: args.pageId,
          title: args.title,
          titleController: args.titleController,
          onTitleChanged: args.onTitleChanged,
          noteTypes: noteTypes,
        ),
        commands: [
          Command(
            id: 'kanban.new',
            label: 'New kanban',
            run: (context) => _newKanban(context, ctx.services.vault(context)),
          ),
        ],
      ),
    );
    ctx.blockKinds.register(
      kanbanInsertBlock(
        createTarget: (context, pageId) =>
            createKanban(ctx.services.vault(context), parent: pageId),
      ),
    );
    ctx.slashItems.register(
      SlashItem(
        id: kanbanInsertKind,
        label: 'Kanban',
        description: 'A board of cards that link to notes.',
        blockKind: kanbanInsertKind,
        run: (_) {},
      ),
    );
  }
}

/// Creates a top-level kanban note and opens it.
Future<void> _newKanban(BuildContext context, Vault vault) async {
  final id = await createKanban(vault);
  if (context.mounted) await openPage(context, id, '', isNew: true);
}
