/** Right-click only removes a seed when Shift is held and the dot is a seed. */
export function mapPointAction(button: number, shift: boolean, isSeed: boolean): 'seed' | 'deselect' | 'play' {
  if (button === 2) return shift && isSeed ? 'seed' : 'deselect';
  return shift ? 'seed' : 'play';
}
