/** Permissions for isolated SSR fixtures only; all transports remain stubs. */
export async function installSyntheticExecutionPolicy(server, allowedActions) {
  const { installExecutionPolicy, validateExecutionPolicy, OFFLINE_EXECUTION_POLICY } =
    await server.ssrLoadModule('/src/executionPolicy.ts');
  const standard = validateExecutionPolicy({
    version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true,
    stage3CreativePackStatus: 'verified_active', allowedActions,
    reasons: ['Synthetic UI fixture permissions; not native/channel acceptance.'],
  });
  installExecutionPolicy(standard);
  return {
    offline() { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); },
    standard() { installExecutionPolicy(standard); },
  };
}
