import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { start } from './lib/state.svelte';
import { speak } from './lib/lang.svelte';

// The TV writes its language into the page, so the first thing shown is already in it.
await speak(document.documentElement.lang);
start();
mount(App, { target: document.getElementById('app')! });
