import 'dart:convert';

import '../../core/models.dart';
import '../../core/vault.dart';
import '../../registry/note_drag_payload.dart';
import 'task_data.dart';
import 'task_day_link.dart';

class CalendarNoteDrop {
  const CalendarNoteDrop({required this.duplicate, this.undo});
  final bool duplicate;
  final Future<void> Function()? undo;
}

// Undo checks current data before writing only the field/block created here.
// The bridge has no compare-and-set or conditional delete: an edit to that
// same field/block arriving between the check and write cannot be detected.

// Serialize this client's drops on the same page so two quick drops cannot
// both pass the duplicate check. Other clients still merge ordinary inserts.
final _pendingDrops = Expando<Map<String, Future<void>>>();

Future<CalendarNoteDrop> dropNoteOnCalendar(
  Vault vault,
  NoteDragPayload payload,
  DateTime day, {
  int? hour,
}) async {
  final pending = _pendingDrops[vault] ??= {};
  final prior = pending[payload.pageId];
  final result = () async {
    if (prior != null) await prior;
    return _drop(vault, payload.pageId, day, hour: hour);
  }();
  final finished = result.then<void>((_) {}, onError: (Object _) {});
  pending[payload.pageId] = finished;
  try {
    return await result;
  } finally {
    if (identical(pending[payload.pageId], finished)) {
      pending.remove(payload.pageId);
    }
  }
}

Future<CalendarNoteDrop> _drop(
  Vault vault,
  String pageId,
  DateTime day, {
  int? hour,
}) async {
  final page = await vault.loadPage(pageId);
  if (page.inTrash) throw StateError('The note is in the trash.');
  if (page.pageType == taskType) {
    final before = await loadTask(vault, pageId);
    final previous = before.dateTime;
    final timed = hour != null || before.hasTime;
    final date = timed
        ? DateTime(
            day.year,
            day.month,
            day.day,
            hour ?? previous?.hour ?? 9,
            hour == null ? previous?.minute ?? 0 : 0,
            hour == null ? previous?.second ?? 0 : 0,
            hour == null ? previous?.millisecond ?? 0 : 0,
            hour == null ? previous?.microsecond ?? 0 : 0,
          )
        : day;
    final appliedDate = taskDateString(date, timed: timed);
    await vault.typeDataSet(pageId, taskAddonId, [
      'date',
    ], jsonEncode(appliedDate));
    await vault.flush();
    await linkTaskToDay(vault, pageId, date);
    return CalendarNoteDrop(
      duplicate: false,
      undo: () async {
        if ((await loadTask(vault, pageId)).date != appliedDate) {
          throw StateError('The task date has changed since this drop.');
        }
        if (before.date == null) {
          await vault.typeDataDelete(pageId, taskAddonId, ['date']);
        } else {
          await vault.typeDataSet(pageId, taskAddonId, [
            'date',
          ], jsonEncode(before.date));
        }
        await vault.flush();
        await linkTaskToDay(vault, pageId, before.dateTime);
      },
    );
  }
  final iso = taskDateString(day);
  if (page.blocks.any(
    (b) => b.marks.any((m) => m.kind == BMarkKind.date && m.url == iso),
  )) {
    return const CalendarNoteDrop(duplicate: true);
  }
  final block = newId();
  final text = '@$iso';
  final mark = BMark(
    start: 0,
    end: text.length,
    kind: BMarkKind.date,
    url: iso,
  );
  // Insert only a new top-level paragraph. Existing text/marks never enter a
  // write, even when the note is open in an editor or another client.
  await vault.apply(pageId, [
    BOp(
      op: OpKind.insert,
      id: block,
      after: page.blocks.where((b) => b.depth == 0).lastOrNull?.id,
      kind: Kinds.paragraph,
      text: text,
      marks: [mark],
      checked: false,
    ),
  ]);
  await vault.flush();
  return CalendarNoteDrop(
    duplicate: false,
    undo: () async {
      final blocks = (await vault.loadPage(pageId)).blocks;
      final at = blocks.indexWhere((b) => b.id == block);
      if (at < 0) return;
      final current = blocks[at];
      final hasChildren =
          at + 1 < blocks.length && blocks[at + 1].depth > current.depth;
      if (hasChildren ||
          current.text != text ||
          current.kind != Kinds.paragraph ||
          current.checked ||
          current.depth != 0 ||
          current.target != null ||
          current.color != null ||
          current.width != null ||
          current.marks.length != 1 ||
          current.marks.single != mark) {
        throw StateError('The added paragraph has changed since this drop.');
      }
      await vault.apply(pageId, [
        BOp(
          op: OpKind.delete,
          id: block,
          kind: '',
          text: '',
          marks: const [],
          checked: false,
        ),
      ]);
      await vault.flush();
    },
  );
}
