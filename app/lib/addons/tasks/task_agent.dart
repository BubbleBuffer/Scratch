// A task assigned to the agent: assigning it starts one run of the agent
// for the task, on the synced default runner (this computer, or a hub job;
// phones use the Hub). The run is linked to the task ([taskRunKey]) and
// answers into the task's page, where its placeholder shows its status.
// Reassigning never starts another run; unassigning a run that has not
// ended cancels it.

import 'dart:convert';

import '../../core/vault.dart';
import '../../services/addon_services.dart';
import 'task_data.dart';

/// What the agent is asked for a task titled [title].
String taskAgentPrompt(String title) {
  final t = title.trim();
  return t.isEmpty
      ? 'Do the task described on this page.'
      : 'Do this task: $t\n\nThe task is this page; its notes are the details.';
}

/// Follows a change of [page]'s assignee from [from] to [to] (after the
/// task was saved). Completes with the id of a run it started.
Future<String?> followTaskAssignee(
  AgentRuns runs,
  Vault vault,
  String page, {
  required String from,
  required String to,
  String title = '',
}) async {
  if (from == to) return null;
  final run = (await loadTask(vault, page)).agentRun;
  if (to == 'agent') {
    if (run != null) return null;
    final id = await runs.ask(taskAgentPrompt(title), page);
    await vault.typeDataSet(page, taskAddonId, [taskRunKey], jsonEncode(id));
    await vault.flush();
    return id;
  }
  if (from == 'agent' && run != null) {
    final t = await runs.run(run);
    if (t == null || AiStatus.active(t.status)) await runs.cancel(run);
  }
  return null;
}

/// The line under the assignee for a task assigned to the agent: its run's
/// status ([run] as this device has it; null while a hub job has not synced
/// here yet).
String taskAgentNote({required bool composer, required TaskData saved, required String assignee, AiTask? run}) {
  if (composer) return 'The agent starts on this task when you create it.';
  if (saved.assignee != 'agent' || assignee != 'agent') {
    return saved.agentRun == null
        ? 'The agent starts on this task when you save.'
        : 'The agent already ran for this task; assigning it again does not start another run.';
  }
  final id = saved.agentRun;
  if (id == null) return 'The agent has not run for this task.';
  if (run == null) return 'The agent is starting on this task.';
  return switch (run.status) {
    AiStatus.queued => 'The agent is queued for this task.',
    AiStatus.running => 'The agent is working on this task.',
    AiStatus.done => 'The agent finished; its answer is on this page.',
    AiStatus.failed => run.error.isEmpty ? 'The agent\'s run failed.' : 'The agent\'s run failed: ${run.error}',
    AiStatus.cancelled => 'The agent\'s run was cancelled.',
    AiStatus.interrupted => 'The agent\'s run was interrupted.',
    _ => 'The agent\'s run: ${run.status}.',
  };
}
