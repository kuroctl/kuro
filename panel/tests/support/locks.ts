// Web Locks en mémoire : jsdom ne fournit pas `navigator.locks`. Les applications montées dans le
// même document le partagent, comme des onglets d'un même navigateur.
const queues = new Map<string, Promise<unknown>>()

const locks = {
  request<T>(name: string, task: () => Promise<T>): Promise<T> {
    const previous = queues.get(name) ?? Promise.resolve()
    const run = previous.then(task)
    queues.set(
      name,
      run.catch(() => undefined),
    )
    return run
  },
}

export function installLocks() {
  Object.defineProperty(navigator, 'locks', { value: locks, configurable: true })
}
