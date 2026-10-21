// The "add a card" panel: type a title for a new note, or pick a note that
// already exists.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../ui/database/widgets.dart' show showPanel;
import '../../ui/ui.dart';

/// What the panel returned: a new card's title, or a request to pick a note.
class KanbanAddChoice {
  const KanbanAddChoice.newCard(this.title) : existing = false;
  const KanbanAddChoice.existing() : title = '', existing = true;

  final String title;
  final bool existing;
}

Future<KanbanAddChoice?> showKanbanAddPanel(BuildContext context) =>
    showPanel<KanbanAddChoice>(
      context,
      globalRectOf(context),
      (_) => const _AddCardPanel(),
      width: 280,
    );

class _AddCardPanel extends StatefulWidget {
  const _AddCardPanel();

  @override
  State<_AddCardPanel> createState() => _AddCardPanelState();
}

class _AddCardPanelState extends State<_AddCardPanel> {
  final _title = TextEditingController();
  final _focus = FocusNode();

  @override
  void dispose() {
    _title.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _create(String value) {
    // An empty title makes no note; the panel stays open for a title.
    final title = value.trim();
    if (title.isEmpty) return;
    Navigator.of(context).pop(KanbanAddChoice.newCard(title));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 4, 4, 4),
          child: InputBox(
            child: PlainField(
              key: const ValueKey('kanban-card-title'),
              controller: _title,
              focusNode: _focus,
              style: ts(context.type.ui, c.textPrimary),
              placeholder: 'New card title',
              autofocus: true,
              textInputAction: TextInputAction.done,
              onSubmitted: _create,
            ),
          ),
        ),
        MenuItem(
          key: const ValueKey('kanban-add-existing'),
          label: 'Add existing note…',
          onTap: () =>
              Navigator.of(context).pop(const KanbanAddChoice.existing()),
        ),
      ],
    );
  }
}
