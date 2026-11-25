<?php

declare(strict_types=1);

namespace Fixture\Tests;

use PHPUnit\Framework\TestCase;

/**
 * Noms de jeux de données piégés pour les filtres `--filter` : tous réussissent, on
 * vérifie qu'un filtre bien échappé en désigne exactement un.
 * Suite « tricky », à lancer seule.
 */
final class TrickyNamesTest extends TestCase
{
    /**
     * @dataProvider trickyProvider
     */
    public function testTrickyNames(string $name): void
    {
        $this->assertNotSame('', $name);
    }

    /** Un nom de méthode qui est le début d'un autre. */
    public function testTricky(): void
    {
        $this->assertTrue(true);
    }

    public static function trickyProvider(): array
    {
        $names = [
            'simple',
            'zéro plus zéro',
            "l'apostrophe | la barre [et] les crochets",
            'des "guillemets" doubles',
            'antislash \\ et slash /',
            'dièse #1 et arobase @x',
            'regex (a|b)+ .* ^$ {2} ?',
            ".*",
            "ligne 1\nligne 2",
            '日本語',
            'avec  deux  espaces',
        ];
        $sets = [];
        foreach ($names as $name) {
            $sets[$name] = [$name];
        }
        return $sets;
    }
}
