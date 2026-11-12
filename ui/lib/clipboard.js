// Copying a chat line or command, with the button saying it worked.

export async function copyText(text, button) {
  try {
    await navigator.clipboard.writeText(text);
    if (button) {
      const was = button.textContent;
      button.textContent = 'Copied ✓';
      setTimeout(() => (button.textContent = was), 1500);
    }
  } catch {
    if (button) button.textContent = 'Copy failed';
  }
}
