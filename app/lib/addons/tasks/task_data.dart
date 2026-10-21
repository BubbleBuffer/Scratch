import 'dart:convert';

import '../../core/models.dart';
import '../../core/vault.dart';
import 'task_day_link.dart';

const taskType = 'task';
const taskAddonId = 'tasks';
const taskStatuses = ['todo', 'doing', 'done'];

/// The type data key naming the agent's run for a task assigned to it
/// (an AI tasks row or hub job id; see task_agent.dart).
const taskRunKey = 'agent_run';

String taskDateString(DateTime date, {bool timed = false}) => timed
    ? date.toUtc().toIso8601String()
    : '${date.year.toString().padLeft(4, '0')}-${date.month.toString().padLeft(2, '0')}-${date.day.toString().padLeft(2, '0')}';

/// Tasks keep their body in ordinary blocks and only scheduling data here.
class TaskData {
  const TaskData({
    this.status = 'todo',
    this.checkbox = true,
    this.date,
    this.durationMin,
    this.assignee = 'me',
    this.calendar,
    this.agentRun,
  });

  final String status, assignee;
  final bool checkbox;
  final String? date, calendar;

  /// The agent's run for this task ([taskRunKey]); only task_agent.dart
  /// writes it, so [toJson] leaves it out.
  final String? agentRun;
  final int? durationMin;
  bool get done => status == 'done';
  bool get hasTime => date?.contains('T') ?? false;
  DateTime? get dateTime {
    final value = date;
    if (value == null) return null;
    final parsed = DateTime.tryParse(value);
    return hasTime ? parsed?.toLocal() : parsed;
  }

  factory TaskData.fromJson(String json) {
    final value = jsonDecode(json);
    if (value is! Map<String, dynamic>) return const TaskData();
    return TaskData(
      status: taskStatuses.contains(value['status'])
          ? value['status'] as String
          : 'todo',
      checkbox: value['checkbox'] is bool ? value['checkbox'] as bool : true,
      date: value['date'] is String ? value['date'] as String : null,
      durationMin:
          value['duration_min'] is int && (value['duration_min'] as int) >= 0
          ? value['duration_min'] as int
          : null,
      assignee: value['assignee'] == 'agent' ? 'agent' : 'me',
      calendar: value['calendar'] is String
          ? value['calendar'] as String
          : null,
      agentRun: value[taskRunKey] is String
          ? value[taskRunKey] as String
          : null,
    );
  }

  Map<String, Object?> toJson() => {
    'status': status,
    'checkbox': checkbox,
    'date': date,
    'duration_min': hasTime ? durationMin : null,
    'assignee': assignee,
    'calendar': calendar,
  };

  AgendaItem agendaItem(String page, String title) {
    final start = dateTime ?? DateTime(1970);
    final stamp = hasTime
        ? start.millisecondsSinceEpoch
        : DateTime.utc(
            start.year,
            start.month,
            start.day,
          ).millisecondsSinceEpoch;
    return AgendaItem(
      page: page,
      title: title,
      kind: AgendaKind.task,
      startMs: stamp,
      endMs: hasTime ? stamp + (durationMin ?? 0) * 60000 : stamp,
      hasTime: hasTime,
      done: done,
      status: status,
      calendar: calendar,
      assignee: assignee,
      updatedMs: 0,
    );
  }
}

Future<TaskData> loadTask(Vault vault, String page) async =>
    TaskData.fromJson(await vault.typeData(page, taskAddonId));

/// Write known keys independently so unrelated and future type data survives.
Future<void> saveTask(Vault vault, String page, TaskData task) async {
  final raw = jsonDecode(await vault.typeData(page, taskAddonId));
  final existing = raw is Map<String, dynamic> ? raw : <String, dynamic>{};
  for (final entry in task.toJson().entries) {
    if (entry.value == null) {
      if (existing.containsKey(entry.key)) {
        await vault.typeDataDelete(page, taskAddonId, [entry.key]);
      }
    } else if (existing[entry.key] != entry.value) {
      await vault.typeDataSet(page, taskAddonId, [
        entry.key,
      ], jsonEncode(entry.value));
    }
  }
  await vault.flush();
  await linkTaskToDay(vault, page, task.dateTime);
}

Future<void> rescheduleTask(
  Vault vault,
  String page,
  DateTime date, {
  int? hour,
  int? minute,
}) async {
  final old = await loadTask(vault, page);
  final previous = old.dateTime;
  final timed = hour != null || old.hasTime;
  final start = timed
      ? DateTime(
          date.year,
          date.month,
          date.day,
          hour ?? previous?.hour ?? 9,
          minute ?? previous?.minute ?? 0,
          previous?.second ?? 0,
          previous?.millisecond ?? 0,
          previous?.microsecond ?? 0,
        )
      : date;
  await vault.typeDataSet(page, taskAddonId, [
    'date',
  ], jsonEncode(taskDateString(start, timed: timed)));
  await vault.flush();
  await linkTaskToDay(vault, page, start);
}
