// Terminal simulé : xterm.js a besoin d'un vrai rendu ; ici la sortie s'accumule dans un <pre>
// lisible par les tests.
export class Terminal {
  options: Record<string, unknown> = {}
  textarea: HTMLTextAreaElement | undefined
  private output: HTMLPreElement | undefined

  loadAddon() {}

  open(parent: HTMLElement) {
    this.output = document.createElement('pre')
    this.output.dataset.testid = 'terminal'
    parent.append(this.output)
  }

  write(text: string) {
    if (this.output) this.output.textContent += text
  }

  dispose() {}
}

export class FitAddon {
  fit() {}
  activate() {}
  dispose() {}
}
