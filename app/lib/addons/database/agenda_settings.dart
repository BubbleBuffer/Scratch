import '../../core/models.dart';

/// Match the core: only editable Date properties make an agenda date.
AgendaRole resolvedAgendaRole(List<PropDef> schema, AgendaRole role) {
  if (role != AgendaRole.auto) return role;
  if (!schema.any((p) => p.kind == PropKind.date)) return AgendaRole.off;
  return schema.any((p) => p.kind == PropKind.status || p.kind == PropKind.checkbox)
      ? AgendaRole.tasks : AgendaRole.events;
}

String agendaRoleLabel(AgendaRole role) => switch (role) {
  AgendaRole.auto => 'Auto', AgendaRole.tasks => 'Tasks',
  AgendaRole.events => 'Events', AgendaRole.off => 'Off',
};

String agendaSettingsLabel(List<PropDef> schema, AgendaSettings settings) => settings.role == AgendaRole.auto
    ? 'Auto · ${agendaRoleLabel(resolvedAgendaRole(schema, settings.role))}' : agendaRoleLabel(settings.role);
