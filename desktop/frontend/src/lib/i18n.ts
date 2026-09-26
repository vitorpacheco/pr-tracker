import english from '../../../../internal/i18n/en.json';

export type Language = 'en' | 'pt';

export function matchLanguage(locale: string): Language {
  return locale
    .trim()
    .toLowerCase()
    .split(/[-_.@]/)[0] === 'pt'
    ? 'pt'
    : 'en';
}

export function browserLanguage(): Language {
  return matchLanguage(navigator.languages?.[0] || navigator.language || 'en');
}

export function translator(language: string) {
  return (message: string): string =>
    language === 'pt'
      ? message
      : ((english as Record<string, string>)[message] ?? message);
}

export function statusLabel(language: string, state: string): string {
  const labels: Record<string, string> = {
    success: 'sucesso',
    failure: 'falhou',
    pending: 'em andamento',
    canceled: 'cancelado',
    approved: 'aprovado',
    changes_requested: 'alterações solicitadas',
    review_required: 'aguardando revisão',
    commented: 'comentou',
    dismissed: 'review descartado',
  };
  return translator(language)(labels[state] ?? state);
}
