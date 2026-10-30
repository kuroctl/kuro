import 'vue-sonner/style.css'
import './assets/main.css'

import { createApp } from 'vue'

import App from './App.vue'
import { installApp } from './bootstrap'

const app = createApp(App)
installApp(app)
app.mount('#app')
