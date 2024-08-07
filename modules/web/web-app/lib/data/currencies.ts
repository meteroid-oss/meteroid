export interface Currency {
  ref: string
  symbol: string
  full: string
  precision: number
}

export const CURRENCIES: Record<string, Currency> = {
  AED: {
    ref: 'AED',
    symbol: 'د.إ;',
    full: 'UAE dirham (AED)',
    precision: 2,
  },
  AFN: {
    ref: 'AFN',
    symbol: 'Afs',
    full: 'Afghan afghani (AFN)',
    precision: 2,
  },
  ALL: {
    ref: 'ALL',
    symbol: 'L',
    full: 'Albanian lek (ALL)',
    precision: 2,
  },
  AMD: {
    ref: 'AMD',
    symbol: 'AMD',
    full: 'Armenian dram (AMD)',
    precision: 2,
  },
  ANG: {
    ref: 'ANG',
    symbol: 'NAƒ',
    full: 'Netherlands Antillean gulden (ANG)',
    precision: 2,
  },
  AOA: {
    ref: 'AOA',
    symbol: 'Kz',
    full: 'Angolan kwanza (AOA)',
    precision: 2,
  },
  ARS: {
    ref: 'ARS',
    symbol: '$',
    full: 'Argentine peso (ARS)',
    precision: 2,
  },
  AUD: {
    ref: 'AUD',
    symbol: '$',
    full: 'Australian dollar (AUD)',
    precision: 2,
  },
  AWG: {
    ref: 'AWG',
    symbol: 'ƒ',
    full: 'Aruban florin (AWG)',
    precision: 2,
  },
  AZN: {
    ref: 'AZN',
    symbol: 'AZN',
    full: 'Azerbaijani manat (AZN)',
    precision: 2,
  },
  BAM: {
    ref: 'BAM',
    symbol: 'KM',
    full: 'Bosnia and Herzegovina konvertibilna marka (BAM)',
    precision: 2,
  },
  BBD: {
    ref: 'BBD',
    symbol: 'Bds$',
    full: 'Barbadian dollar (BBD)',
    precision: 2,
  },
  BDT: {
    ref: 'BDT',
    symbol: '৳',
    full: 'Bangladeshi taka (BDT)',
    precision: 2,
  },
  BGN: {
    ref: 'BGN',
    symbol: 'BGN',
    full: 'Bulgarian lev (BGN)',
    precision: 2,
  },
  BHD: {
    ref: 'BHD',
    symbol: '.د.ب',
    full: 'Bahraini dinar (BHD)',
    precision: 3,
  },
  BIF: {
    ref: 'BIF',
    symbol: 'FBu',
    full: 'Burundi franc (BIF)',
    precision: 0,
  },
  BMD: {
    ref: 'BMD',
    symbol: 'BD$',
    full: 'Bermudian dollar (BMD)',
    precision: 2,
  },
  BND: {
    ref: 'BND',
    symbol: 'B$',
    full: 'Brunei dollar (BND)',
    precision: 2,
  },
  BOB: {
    ref: 'BOB',
    symbol: 'Bs.',
    full: 'Bolivian boliviano (BOB)',
    precision: 2,
  },
  BRL: {
    ref: 'BRL',
    symbol: 'R$',
    full: 'Brazilian real (BRL)',
    precision: 2,
  },
  BSD: {
    ref: 'BSD',
    symbol: 'B$',
    full: 'Bahamian dollar (BSD)',
    precision: 2,
  },
  BTN: {
    ref: 'BTN',
    symbol: 'Nu.',
    full: 'Bhutanese ngultrum (BTN)',
    precision: 2,
  },
  BWP: {
    ref: 'BWP',
    symbol: 'P',
    full: 'Botswana pula (BWP)',
    precision: 2,
  },
  BYR: {
    ref: 'BYR',
    symbol: 'Br',
    full: 'Belarusian ruble (BYR)',
    precision: 2,
  },
  BZD: {
    ref: 'BZD',
    symbol: 'BZ$',
    full: 'Belize dollar (BZD)',
    precision: 2,
  },
  CAD: {
    ref: 'CAD',
    symbol: '$',
    full: 'Canadian dollar (CAD)',
    precision: 2,
  },
  CDF: {
    ref: 'CDF',
    symbol: 'F',
    full: 'Congolese franc (CDF)',
    precision: 2,
  },
  CHF: {
    ref: 'CHF',
    symbol: 'Fr.',
    full: 'Swiss franc (CHF)',
    precision: 2,
  },
  CLP: {
    ref: 'CLP',
    symbol: '$',
    full: 'Chilean peso (CLP)',
    precision: 0,
  },
  CNY: {
    ref: 'CNY',
    symbol: '¥',
    full: 'Chinese/Yuan renminbi (CNY)',
    precision: 2,
  },
  COP: {
    ref: 'COP',
    symbol: 'Col$',
    full: 'Colombian peso (COP)',
    precision: 2,
  },
  CRC: {
    ref: 'CRC',
    symbol: '₡',
    full: 'Costa Rican colon (CRC)',
    precision: 2,
  },
  CUC: {
    ref: 'CUC',
    symbol: '$',
    full: 'Cuban peso (CUC)',
    precision: 2,
  },
  CVE: {
    ref: 'CVE',
    symbol: 'Esc',
    full: 'Cape Verdean escudo (CVE)',
    precision: 2,
  },
  CZK: {
    ref: 'CZK',
    symbol: 'Kč',
    full: 'Czech koruna (CZK)',
    precision: 2,
  },
  DJF: {
    ref: 'DJF',
    symbol: 'Fdj',
    full: 'Djiboutian franc (DJF)',
    precision: 0,
  },
  DKK: {
    ref: 'DKK',
    symbol: 'Kr',
    full: 'Danish krone (DKK)',
    precision: 2,
  },
  DOP: {
    ref: 'DOP',
    symbol: 'RD$',
    full: 'Dominican peso (DOP)',
    precision: 2,
  },
  DZD: {
    ref: 'DZD',
    symbol: 'د.ج',
    full: 'Algerian dinar (DZD)',
    precision: 2,
  },
  EEK: {
    ref: 'EEK',
    symbol: 'KR',
    full: 'Estonian kroon (EEK)',
    precision: 2,
  },
  EGP: {
    ref: 'EGP',
    symbol: '£',
    full: 'Egyptian pound (EGP)',
    precision: 2,
  },
  ERN: {
    ref: 'ERN',
    symbol: 'Nfa',
    full: 'Eritrean nakfa (ERN)',
    precision: 2,
  },
  ETB: {
    ref: 'ETB',
    symbol: 'Br',
    full: 'Ethiopian birr (ETB)',
    precision: 2,
  },
  EUR: {
    ref: 'EUR',
    symbol: '€',
    full: 'Euro (EUR)',
    precision: 2,
  },
  FJD: {
    ref: 'FJD',
    symbol: 'FJ$',
    full: 'Fijian dollar (FJD)',
    precision: 2,
  },
  FKP: {
    ref: 'FKP',
    symbol: '£',
    full: 'Falkland Islands pound (FKP)',
    precision: 2,
  },
  GBP: {
    ref: 'GBP',
    symbol: '£',
    full: 'British pound (GBP)',
    precision: 2,
  },
  GEL: {
    ref: 'GEL',
    symbol: 'GEL',
    full: 'Georgian lari (GEL)',
    precision: 2,
  },
  GHS: {
    ref: 'GHS',
    symbol: 'GH₵',
    full: 'Ghanaian cedi (GHS)',
    precision: 2,
  },
  GIP: {
    ref: 'GIP',
    symbol: '£',
    full: 'Gibraltar pound (GIP)',
    precision: 2,
  },
  GMD: {
    ref: 'GMD',
    symbol: 'D',
    full: 'Gambian dalasi (GMD)',
    precision: 2,
  },
  GNF: {
    ref: 'GNF',
    symbol: 'FG',
    full: 'Guinean franc (GNF)',
    precision: 0,
  },
  GQE: {
    ref: 'GQE',
    symbol: 'CFA',
    full: 'Central African CFA franc (GQE)',
    precision: 2,
  },
  GTQ: {
    ref: 'GTQ',
    symbol: 'Q',
    full: 'Guatemalan quetzal (GTQ)',
    precision: 2,
  },
  GYD: {
    ref: 'GYD',
    symbol: 'GY$',
    full: 'Guyanese dollar (GYD)',
    precision: 2,
  },
  HKD: {
    ref: 'HKD',
    symbol: 'HK$',
    full: 'Hong Kong dollar (HKD)',
    precision: 2,
  },
  HNL: {
    ref: 'HNL',
    symbol: 'L',
    full: 'Honduran lempira (HNL)',
    precision: 2,
  },
  HRK: {
    ref: 'HRK',
    symbol: 'kn',
    full: 'Croatian kuna (HRK)',
    precision: 2,
  },
  HTG: {
    ref: 'HTG',
    symbol: 'G',
    full: 'Haitian gourde (HTG)',
    precision: 2,
  },
  HUF: {
    ref: 'HUF',
    symbol: 'Ft',
    full: 'Hungarian forint (HUF)',
    precision: 2,
  },
  IDR: {
    ref: 'IDR',
    symbol: 'Rp',
    full: 'Indonesian rupiah (IDR)',
    precision: 2,
  },
  ILS: {
    ref: 'ILS',
    symbol: '₪',
    full: 'Israeli new sheqel (ILS)',
    precision: 2,
  },
  INR: {
    ref: 'INR',
    symbol: '₹',
    full: 'Indian rupee (INR)',
    precision: 2,
  },
  IQD: {
    ref: 'IQD',
    symbol: 'د.ع',
    full: 'Iraqi dinar (IQD)',
    precision: 2,
  },
  IRR: {
    ref: 'IRR',
    symbol: 'IRR',
    full: 'Iranian rial (IRR)',
    precision: 2,
  },
  ISK: {
    ref: 'ISK',
    symbol: 'kr',
    full: 'Icelandic króna (ISK)',
    precision: 2,
  },
  JMD: {
    ref: 'JMD',
    symbol: 'J$',
    full: 'Jamaican dollar (JMD)',
    precision: 2,
  },
  JOD: {
    ref: 'JOD',
    symbol: 'JOD',
    full: 'Jordanian dinar (JOD)',
    precision: 3,
  },
  JPY: {
    ref: 'JPY',
    symbol: '¥',
    full: 'Japanese yen (JPY)',
    precision: 0,
  },
  KES: {
    ref: 'KES',
    symbol: 'KSh',
    full: 'Kenyan shilling (KES)',
    precision: 2,
  },
  KGS: {
    ref: 'KGS',
    symbol: 'сом',
    full: 'Kyrgyzstani som (KGS)',
    precision: 2,
  },
  KHR: {
    ref: 'KHR',
    symbol: '៛',
    full: 'Cambodian riel (KHR)',
    precision: 2,
  },
  KMF: {
    ref: 'KMF',
    symbol: 'KMF',
    full: 'Comorian franc (KMF)',
    precision: 0,
  },
  KPW: {
    ref: 'KPW',
    symbol: 'W',
    full: 'North Korean won (KPW)',
    precision: 2,
  },
  KRW: {
    ref: 'KRW',
    symbol: 'W',
    full: 'South Korean won (KRW)',
    precision: 0,
  },
  KWD: {
    ref: 'KWD',
    symbol: 'KWD',
    full: 'Kuwaiti dinar (KWD)',
    precision: 3,
  },
  KYD: {
    ref: 'KYD',
    symbol: 'KY$',
    full: 'Cayman Islands dollar (KYD)',
    precision: 2,
  },
  KZT: {
    ref: 'KZT',
    symbol: 'T',
    full: 'Kazakhstani tenge (KZT)',
    precision: 2,
  },
  LAK: {
    ref: 'LAK',
    symbol: 'KN',
    full: 'Lao kip (LAK)',
    precision: 2,
  },
  LBP: {
    ref: 'LBP',
    symbol: '£',
    full: 'Lebanese lira (LBP)',
    precision: 2,
  },
  LKR: {
    ref: 'LKR',
    symbol: 'Rs',
    full: 'Sri Lankan rupee (LKR)',
    precision: 2,
  },
  LRD: {
    ref: 'LRD',
    symbol: 'L$',
    full: 'Liberian dollar (LRD)',
    precision: 2,
  },
  LSL: {
    ref: 'LSL',
    symbol: 'M',
    full: 'Lesotho loti (LSL)',
    precision: 2,
  },
  LTL: {
    ref: 'LTL',
    symbol: 'Lt',
    full: 'Lithuanian litas (LTL)',
    precision: 2,
  },
  LVL: {
    ref: 'LVL',
    symbol: 'Ls',
    full: 'Latvian lats (LVL)',
    precision: 2,
  },
  LYD: {
    ref: 'LYD',
    symbol: 'LD',
    full: 'Libyan dinar (LYD)',
    precision: 2,
  },
  MAD: {
    ref: 'MAD',
    symbol: 'MAD',
    full: 'Moroidan dirham (MAD)',
    precision: 2,
  },
  MDL: {
    ref: 'MDL',
    symbol: 'MDL',
    full: 'Moldovan leu (MDL)',
    precision: 2,
  },
  MGA: {
    ref: 'MGA',
    symbol: 'FMG',
    full: 'Malagasy ariary (MGA)',
    precision: 0,
  },
  MKD: {
    ref: 'MKD',
    symbol: 'MKD',
    full: 'Macedonian denar (MKD)',
    precision: 2,
  },
  MMK: {
    ref: 'MMK',
    symbol: 'K',
    full: 'Myanma kyat (MMK)',
    precision: 2,
  },
  MNT: {
    ref: 'MNT',
    symbol: '₮',
    full: 'Mongolian tugrik (MNT)',
    precision: 2,
  },
  MOP: {
    ref: 'MOP',
    symbol: 'P',
    full: 'Macanese pataca (MOP)',
    precision: 2,
  },
  MRO: {
    ref: 'MRO',
    symbol: 'UM',
    full: 'Mauritanian ouguiya (MRO)',
    precision: 2,
  },
  MUR: {
    ref: 'MUR',
    symbol: 'Rs',
    full: 'Mauritian rupee (MUR)',
    precision: 2,
  },
  MVR: {
    ref: 'MVR',
    symbol: 'Rf',
    full: 'Maldivian rufiyaa (MVR)',
    precision: 2,
  },
  MWK: {
    ref: 'MWK',
    symbol: 'MK',
    full: 'Malawian kwacha (MWK)',
    precision: 2,
  },
  MXN: {
    ref: 'MXN',
    symbol: '$',
    full: 'Mexican peso (MXN)',
    precision: 2,
  },
  MYR: {
    ref: 'MYR',
    symbol: 'RM',
    full: 'Malaysian ringgit (MYR)',
    precision: 2,
  },
  MZM: {
    ref: 'MZM',
    symbol: 'MTn',
    full: 'Mozambican metical (MZM)',
    precision: 2,
  },
  NAD: {
    ref: 'NAD',
    symbol: 'N$',
    full: 'Namibian dollar (NAD)',
    precision: 2,
  },
  NGN: {
    ref: 'NGN',
    symbol: '₦',
    full: 'Nigerian naira (NGN)',
    precision: 2,
  },
  NIO: {
    ref: 'NIO',
    symbol: 'C$',
    full: 'Nicaraguan córdoba (NIO)',
    precision: 2,
  },
  NOK: {
    ref: 'NOK',
    symbol: 'kr',
    full: 'Norwegian krone (NOK)',
    precision: 2,
  },
  NPR: {
    ref: 'NPR',
    symbol: 'NRs',
    full: 'Nepalese rupee (NPR)',
    precision: 2,
  },
  NZD: {
    ref: 'NZD',
    symbol: 'NZ$',
    full: 'New Zealand dollar (NZD)',
    precision: 2,
  },
  OMR: {
    ref: 'OMR',
    symbol: 'OMR',
    full: 'Omani rial (OMR)',
    precision: 3,
  },
  PAB: {
    ref: 'PAB',
    symbol: 'B./',
    full: 'Panamanian balboa (PAB)',
    precision: 2,
  },
  PEN: {
    ref: 'PEN',
    symbol: 'S/.',
    full: 'Peruvian nuevo sol (PEN)',
    precision: 2,
  },
  PGK: {
    ref: 'PGK',
    symbol: 'K',
    full: 'Papua New Guinean kina (PGK)',
    precision: 2,
  },
  PHP: {
    ref: 'PHP',
    symbol: '₱',
    full: 'Philippine peso (PHP)',
    precision: 2,
  },
  PKR: {
    ref: 'PKR',
    symbol: 'Rs.',
    full: 'Pakistani rupee (PKR)',
    precision: 2,
  },
  PLN: {
    ref: 'PLN',
    symbol: 'zł',
    full: 'Polish zloty (PLN)',
    precision: 2,
  },
  PYG: {
    ref: 'PYG',
    symbol: '₲',
    full: 'Paraguayan guarani (PYG)',
    precision: 0,
  },
  QAR: {
    ref: 'QAR',
    symbol: 'QR',
    full: 'Qatari riyal (QAR)',
    precision: 2,
  },
  RON: {
    ref: 'RON',
    symbol: 'L',
    full: 'Romanian leu (RON)',
    precision: 2,
  },
  RSD: {
    ref: 'RSD',
    symbol: 'din.',
    full: 'Serbian dinar (RSD)',
    precision: 2,
  },
  RUB: {
    ref: 'RUB',
    symbol: 'R',
    full: 'Russian ruble (RUB)',
    precision: 2,
  },
  SAR: {
    ref: 'SAR',
    symbol: 'SR',
    full: 'Saudi riyal (SAR)',
    precision: 2,
  },
  SBD: {
    ref: 'SBD',
    symbol: 'SI$',
    full: 'Solomon Islands dollar (SBD)',
    precision: 2,
  },
  SCR: {
    ref: 'SCR',
    symbol: 'SR',
    full: 'Seychellois rupee (SCR)',
    precision: 2,
  },
  SDG: {
    ref: 'SDG',
    symbol: 'SDG',
    full: 'Sudanese pound (SDG)',
    precision: 2,
  },
  SEK: {
    ref: 'SEK',
    symbol: 'kr',
    full: 'Swedish krona (SEK)',
    precision: 2,
  },
  SGD: {
    ref: 'SGD',
    symbol: 'S$',
    full: 'Singapore dollar (SGD)',
    precision: 2,
  },
  SHP: {
    ref: 'SHP',
    symbol: '£',
    full: 'Saint Helena pound (SHP)',
    precision: 2,
  },
  SLL: {
    ref: 'SLL',
    symbol: 'Le',
    full: 'Sierra Leonean leone (SLL)',
    precision: 2,
  },
  SOS: {
    ref: 'SOS',
    symbol: 'Sh.',
    full: 'Somali shilling (SOS)',
    precision: 2,
  },
  SRD: {
    ref: 'SRD',
    symbol: '$',
    full: 'Surinamese dollar (SRD)',
    precision: 2,
  },
  SYP: {
    ref: 'SYP',
    symbol: 'LS',
    full: 'Syrian pound (SYP)',
    precision: 2,
  },
  SZL: {
    ref: 'SZL',
    symbol: 'E',
    full: 'Swazi lilangeni (SZL)',
    precision: 2,
  },
  THB: {
    ref: 'THB',
    symbol: '฿',
    full: 'Thai baht (THB)',
    precision: 2,
  },
  TJS: {
    ref: 'TJS',
    symbol: 'TJS',
    full: 'Tajikistani somoni (TJS)',
    precision: 2,
  },
  TMT: {
    ref: 'TMT',
    symbol: 'm',
    full: 'Turkmen manat (TMT)',
    precision: 2,
  },
  TND: {
    ref: 'TND',
    symbol: 'DT',
    full: 'Tunisian dinar (TND)',
    precision: 3,
  },
  TRY: {
    ref: 'TRY',
    symbol: 'TRY',
    full: 'Turkish new lira (TRY)',
    precision: 2,
  },
  TTD: {
    ref: 'TTD',
    symbol: 'TT$',
    full: 'Trinidad and Tobago dollar (TTD)',
    precision: 2,
  },
  TWD: {
    ref: 'TWD',
    symbol: 'NT$',
    full: 'New Taiwan dollar (TWD)',
    precision: 2,
  },
  TZS: {
    ref: 'TZS',
    symbol: 'TZS',
    full: 'Tanzanian shilling (TZS)',
    precision: 2,
  },
  UAH: {
    ref: 'UAH',
    symbol: 'UAH',
    full: 'Ukrainian hryvnia (UAH)',
    precision: 2,
  },
  UGX: {
    ref: 'UGX',
    symbol: 'USh',
    full: 'Ugandan shilling (UGX)',
    precision: 0,
  },
  USD: {
    ref: 'USD',
    symbol: '$',
    full: 'United States dollar (USD)',
    precision: 2,
  },
  UYU: {
    ref: 'UYU',
    symbol: '$U',
    full: 'Uruguayan peso (UYU)',
    precision: 2,
  },
  UZS: {
    ref: 'UZS',
    symbol: 'UZS',
    full: 'Uzbekistani som (UZS)',
    precision: 2,
  },
  VEB: {
    ref: 'VEB',
    symbol: 'Bs',
    full: 'Venezuelan bolivar (VEB)',
    precision: 2,
  },
  VND: {
    ref: 'VND',
    symbol: '₫',
    full: 'Vietnamese dong (VND)',
    precision: 0,
  },
  VUV: {
    ref: 'VUV',
    symbol: 'VT',
    full: 'Vanuatu vatu (VUV)',
    precision: 0,
  },
  WST: {
    ref: 'WST',
    symbol: 'WS$',
    full: 'Samoan tala (WST)',
    precision: 2,
  },
  XAF: {
    ref: 'XAF',
    symbol: 'CFA',
    full: 'Central African CFA franc (XAF)',
    precision: 0,
  },
  XCD: {
    ref: 'XCD',
    symbol: 'EC$',
    full: 'East Caribbean dollar (XCD)',
    precision: 2,
  },
  XDR: {
    ref: 'XDR',
    symbol: 'SDR',
    full: 'Special Drawing Rights (XDR)',
    precision: 2,
  },
  XOF: {
    ref: 'XOF',
    symbol: 'CFA',
    full: 'West African CFA franc (XOF)',
    precision: 0,
  },
  XPF: {
    ref: 'XPF',
    symbol: 'F',
    full: 'CFP franc (XPF)',
    precision: 0,
  },
  YER: {
    ref: 'YER',
    symbol: 'YER',
    full: 'Yemeni rial (YER)',
    precision: 2,
  },
  ZAR: {
    ref: 'ZAR',
    symbol: 'R',
    full: 'South African rand (ZAR)',
    precision: 2,
  },
  ZMK: {
    ref: 'ZMK',
    symbol: 'ZK',
    full: 'Zambian kwacha (ZMK)',
    precision: 2,
  },
  ZWR: {
    ref: 'ZWR',
    symbol: 'Z$',
    full: 'Zimbabwean dollar (ZWR)',
    precision: 2,
  },
}
