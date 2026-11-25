<?php
namespace App\Service;

use Acme\Logger;
use App\Model\User;
use App\Model\Admin as Boss;
use function App\format_name;
use const App\VERSION;

class UserService
{
    private User $current;

    public function __construct(private Logger $logger)
    {
        $this->current = User::create('root');
    }

    public function rename(User $user, $other): string
    {
        // getName dans un commentaire
        $label = 'getName';
        $user->touch();
        $user->save();
        $fresh = new User('a');
        $boss = new Boss('b');
        $this->logger->log(VERSION);
        legacy_helper();
        strlen($label);
        self::check();
        static::check();
        echo User::ROLE_ADMIN;
        return format_name($user->getName())
            . $this->current->getName()
            . $fresh->getName()
            . $boss->getName()
            . $other->getName();
    }

    private static function check(): void
    {
    }
}
