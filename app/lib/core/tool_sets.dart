/// Tool sets: what an agent token, a schedule or an in-app runner may use.
/// The same built-in sets as the Rust registry (`vault_tools::ToolSet`);
/// Everything is always offered.
library;

/// The built-in tool sets' stored names, narrowest first.
const toolSets = ['read', 'write', 'everything'];

/// The set new agent tokens get unless another is picked.
const defaultTokenToolSet = 'write';

/// The set an in-app agent runner has until one is picked.
const defaultAgentToolSet = 'everything';

String toolSetLabel(String set) => switch (set) {
  'read' => 'Read only',
  'write' => 'Read and write',
  'everything' => 'Everything',
  _ => set,
};

/// One line on what a set covers.
String toolSetDescription(String set) => switch (set) {
  'read' => 'Reads the vault and changes nothing.',
  'write' => 'Every vault tool.',
  'everything' => 'Every tool the surface serves, including tools added later.',
  _ => '',
};
