import { consoleLog, installConsoleCapture } from './consoleLog'

const dispose = installConsoleCapture(console, (level, message) => consoleLog.append(level, 'App', message))
if (import.meta.hot) import.meta.hot.dispose(dispose)
