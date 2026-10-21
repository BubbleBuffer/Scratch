// Making schedule notes. A plain schedule has no step yet. The weekly note
// schedule is the first built-in: Mondays at 08:00 on the device's zone, a
// "Weekly note" template under the Journal, and the available hub as runner.

import 'dart:convert';

import '../../core/vault.dart';
import 'backend.dart';
import 'format.dart';
import 'storage.dart';

/// Title of the template the weekly schedule creates from.
const weeklyTemplateTitle = 'Weekly note';

/// Title of the weekly schedule note.
const weeklyScheduleTitle = 'Weekly note schedule';

/// The title template of a weekly note (rendered per occurrence).
const weeklyNoteTitle = 'Week {{week}} {{year}}';

/// Creates a plain schedule under [parent] (top level when null) and returns
/// its id. It has no step and no runner yet, so it does not run.
Future<String> createSchedule(
  Vault vault, {
  String? parent,
  String title = 'Schedule',
  DateTime? now,
  required String deviceTimeZone,
}) async {
  final clock = now ?? DateTime.now();
  final id = await vault.createPage(
    parent: parent,
    title: title,
    pageType: scheduleType,
  );
  final def = ScheduleDef.blank(
    tz: deviceTimeZone,
    at: localAtString(nextMonday(clock).add(const Duration(hours: 8))),
  );
  await _writeDefinition(vault, id, def);
  return id;
}

/// Creates the weekly note schedule under [parent]. It creates the "Weekly
/// note" template under the Journal when that is missing. [nodes] decides
/// the runner: the first available hub, or none.
Future<String> createWeeklySchedule(
  Vault vault, {
  String? parent,
  required List<ScheduleNode> nodes,
  DateTime? now,
  required String deviceTimeZone,
}) async {
  final clock = now ?? DateTime.now();
  final journal = await findJournal(vault);
  final template = journal == null
      ? null
      : await weeklyTemplate(vault, journal);
  final hub = nodes.where((n) => n.kind == 'hub' && n.available).isEmpty
      ? null
      : nodes.firstWhere((n) => n.kind == 'hub' && n.available);
  final id = await vault.createPage(
    parent: parent,
    title: weeklyScheduleTitle,
    pageType: scheduleType,
  );
  final def = ScheduleDef(
    trigger: ScheduleTrigger(
      rrule: 'FREQ=WEEKLY;BYDAY=MO',
      at: localAtString(nextMonday(clock).add(const Duration(hours: 8))),
      tz: deviceTimeZone,
      missed: missedLatest,
    ),
    runner: hub?.id,
    toolSet: 'write',
    steps: [
      ScheduleStep(
        id: 'note',
        kind: scheduleNoteStepKind,
        template: template == null ? null : pageLink(template),
        parent: journal == null ? null : pageLink(journal),
        title: weeklyNoteTitle,
      ),
    ],
  );
  await _writeDefinition(vault, id, def);
  return id;
}

/// The Journal page: the top-level database titled "Journal". Null when the
/// vault has none.
Future<String?> findJournal(Vault vault) async {
  final rows = await vault.listChildren(null, limit: 500);
  for (final row in rows) {
    if (row.title == 'Journal' && row.pageType == 'database') return row.id;
  }
  for (final row in rows) {
    if (row.title == 'Journal') return row.id;
  }
  return null;
}

/// The "Weekly note" template under [journal], created when missing.
Future<String> weeklyTemplate(Vault vault, String journal) async {
  final templates = await vault.run(
    (v) => v.listTemplates(parent: journal),
  );
  for (final t in templates) {
    if (t.title == weeklyTemplateTitle) return t.id;
  }
  return vault.run(
    (v) => v.createTemplate(parent: journal, title: weeklyTemplateTitle),
    writes: true,
  );
}

/// The next Monday strictly after [from]'s date, at midnight local time.
DateTime nextMonday(DateTime from) {
  final days = (DateTime.monday - from.weekday + 7) % 7;
  final day = DateTime(from.year, from.month, from.day);
  return day.add(Duration(days: days == 0 ? 7 : days));
}

Future<void> _writeDefinition(
  Vault vault,
  String id,
  ScheduleDef def,
) async {
  final storage = VaultScheduleStorage(vault);
  for (final entry in def.toJson().entries) {
    await storage.setKey(id, scheduleAddonId, entry.key, jsonEncode(entry.value));
  }
}

