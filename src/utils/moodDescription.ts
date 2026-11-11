/** UI fields share one signed representation with the analyzer. Commas remain
 * inside descriptions; only a leading '- ' changes the direction. */
export function splitDescription(text: string): { more: string; less: string } {
  const more: string[] = [], less: string[] = [];
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith('- ')) less.push(line.slice(2).trim());
    else more.push(line.replace(/^\+ /, ''));
  }
  return { more: more.join('\n'), less: less.join('\n') };
}

export function joinDescription(more: string, less: string): string {
  const lines = (text: string) => text.split('\n').map(s => s.trim()).filter(Boolean);
  return [...lines(more), ...lines(less).map(s => `- ${s}`)].join('\n');
}
