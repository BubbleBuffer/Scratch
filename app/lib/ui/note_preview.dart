import 'package:flutter/widgets.dart';

import '../registry/note_type_registry.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import 'ui.dart';

/// A compact note reference; add-ons can use it unchanged or supply their own.
Widget noteChip(BuildContext context, NoteArgs args) => Hover(
  onTap: args.onOpen,
  child: Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      PageIcon(icon: args.icon, size: 16, color: context.colors.iconSecondary),
      const SizedBox(width: 6),
      Flexible(
        child: Text(
          displayTitle(args.title, firstLine: AppScope.maybeOf(context) == null
              ? '' : context.watch<PagesStore>().meta(args.pageId)?.firstLine ?? ''),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: ts(context.type.ui, context.colors.textPrimary),
        ),
      ),
    ],
  ),
);

Widget noteCard(BuildContext context, NoteArgs args) =>
    Padding(padding: const EdgeInsets.all(12), child: noteChip(context, args));
