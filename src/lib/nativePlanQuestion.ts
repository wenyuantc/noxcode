export const PLAN_QUESTION_OTHER = "__other__";

export function resolvePlanQuestionAnswer(
  options: string[],
  selected: string,
  otherText: string,
): string | null {
  const custom = otherText.trim();
  if (options.length < 2) {
    return custom.length > 0 ? custom : null;
  }
  if (selected === PLAN_QUESTION_OTHER) {
    return custom.length > 0 ? custom : null;
  }
  const choice = selected.trim();
  if (options.includes(choice)) {
    return choice;
  }
  return null;
}
