import '../../core/models.dart';
import '../../core/vault.dart';
import 'task_data.dart';

/// Block dates need an atomic date edit, which the current bridge does not
/// expose. A whole-block SetText from a loaded snapshot can lose other edits.
/// Mentions remain references; database rows have an independent date cell.
bool canRescheduleCalendarItem(AgendaItem item) =>
    item.block == null &&
    (item.collection != null
        ? item.kind == AgendaKind.task || item.kind == AgendaKind.event
        : item.kind == AgendaKind.task);

Future<void> rescheduleCalendarItem(
  Vault vault,
  AgendaItem item,
  DateTime day, {
  int? hour,
}) async {
  if (!canRescheduleCalendarItem(item)) {
    throw StateError('This calendar reference cannot be rescheduled.');
  }
  final page = await vault.loadPage(item.page);
  if (page.inTrash) throw StateError('The page is in the trash.');
  final collection = item.collection;
  if (collection == null) {
    if (page.pageType != taskType) {
      throw StateError('The page is no longer a task.');
    }
    return rescheduleTask(vault, item.page, day, hour: hour);
  }
  if (page.parent != collection) {
    throw StateError('The row is no longer in this database.');
  }
  final database = await vault.loadPage(collection);
  if (database.inTrash || database.pageType != databaseType) {
    throw StateError('The database is no longer available.');
  }
  final schema = await vault.run((v) => v.getSchema(collection: collection));
  final settings = await vault.run(
    (v) => v.agendaSettings(collection: collection),
  );
  // Match the core's date_def: the configured property, then the first Date
  // if that key no longer exists. Do not assume the first Date is configured.
  final date =
      schema.where((p) => p.key == settings.dateProp).firstOrNull ??
      schema.where((p) => p.kind == PropKind.date).firstOrNull;
  if (date == null || date.kind != PropKind.date) {
    throw StateError('The database has no editable agenda date.');
  }
  final old = page.properties
      .where((p) => p.key == date.key)
      .firstOrNull
      ?.value;
  if (old == null || old.kind != ValueKind.date) {
    throw StateError('The row no longer has an agenda date.');
  }
  final timed = hour != null || old.hasTime;
  final previous = DateTime.fromMillisecondsSinceEpoch(
    old.startMs.toInt(),
    isUtc: !old.hasTime,
  );
  final start = timed
      ? DateTime(
          day.year,
          day.month,
          day.day,
          hour ?? previous.hour,
          previous.minute,
          previous.second,
          previous.millisecond,
        ).millisecondsSinceEpoch
      : DateTime.utc(day.year, day.month, day.day).millisecondsSinceEpoch;
  final delta = start - old.startMs.toInt();
  // Only this cell is written. Reading its current value preserves a range
  // changed since the drag began; unrelated properties/blocks are never sent.
  // Like other property edits, simultaneous writes to this same cell are
  // last-writer-wins; the bridge does not provide a conditional property write.
  await vault.run(
    (v) => v.setProperty(
      page: item.page,
      key: date.key,
      value: PropValue(
        kind: ValueKind.date,
        text: '',
        number: 0,
        startMs: start,
        endMs: old.endMs == null ? null : old.endMs!.toInt() + delta,
        hasTime: timed,
        checked: false,
        ids: const [],
      ),
    ),
    writes: true,
  );
  await vault.flush();
}
