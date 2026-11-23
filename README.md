# Optimi-ez

Optimizador de Windows para juegos, escrito en Rust.

> **Estado: inicio del proyecto.** Este repositorio todavía no contiene código que cambie servicios ni ajustes de Windows. No lo uses para aplicar optimizaciones.

## Objetivo

Crear perfiles por juego que identifiquen los servicios de Windows con evidencia de que afectan negativamente la experiencia gamer y permitan deshabilitarlos de forma transparente, segura y reversible. Que un servicio esté activo no significa por sí solo que perjudique el rendimiento: cada recomendación deberá explicar su evidencia y validarse con mediciones.

## Principios de seguridad

- Mostrar cada cambio propuesto, sus posibles efectos y riesgos; pedir confirmación explícita antes de aplicarlo.
- No desactivar servicios críticos ni protecciones de seguridad.
- Guardar el estado original y ofrecer una forma comprobable de restaurarlo antes de habilitar cambios.
- No hacer cambios ocultos ni automáticos en segundo plano.
- Evitar promesas de mejora sin mediciones reproducibles.

## Estado

El repositorio contiene la licencia MIT, la configuración inicial de Git para Rust y el esqueleto de la primera rebanada vertical: una aplicación Tauri 2 que arranca en modo de solo lectura, lee el estado del servicio DiagTrack y el plan de energía activo desde Windows, y muestra la versión de Windows y la cuenta interactiva observadas.

`apply`, `undo` y `restore_all` existen en la superficie de comandos del dominio y responden explícitamente que la escritura todavía no está implementada. La aplicación no escribe en el sistema ni solicita elevación.

La aplicación todavía no aplica optimizaciones. No la uses para cambiar servicios ni ajustes de Windows.

## Licencia

Este proyecto se distribuye bajo la licencia [MIT](LICENSE).
